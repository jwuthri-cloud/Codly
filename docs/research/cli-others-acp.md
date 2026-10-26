# ACP (Agent Client Protocol) and the other agent CLIs: Gemini, OpenCode, Cursor, Copilot, Pi

Research for Codly. Written 2026-10-25. Scope: ACP as a universal adapter layer, plus the headless or native modes of Gemini CLI, OpenCode, Cursor CLI, GitHub Copilot CLI and Pi. Claude Code and Codex native modes are covered in their own reports. Their fixtures sit in the same `fixtures/` folder (`claude-*.jsonl`, `codex-*.jsonl`).

**Evidence legend** (used throughout):

- **[V-local]**: I ran it on this Mac today. The output is in `fixtures/`.
- **[V-src]**: I read it in source code or a JSON schema (the commit or version is named).
- **[V-docs]**: I read it in official docs or a changelog fetched today.
- **[Claim]**: the vendor or project says so. I could not check it.
- **[Unverified]**: my inference, or something I could not confirm.

Local versions: `gemini` 0.62.0 (npm latest is 0.65.0), `opencode` 1.18.31 (latest is 1.19.4), `claude` 2.1.309, `codex` 0.161.0, `node` 22.23.3. `cursor-agent`/`agent`, `copilot` and `pi` are **not installed**. Those sections come from docs and source only.

---

## 0. TL;DR (decision-relevant)

1. **ACP v1 is stable, widely adopted and still moving fast.** The schema is at **v1.26.0** (2026-10-16), and the wire `protocolVersion` is the integer **`1`**. A **v2 draft** was published on 2026-07-20. Clients ask for it by sending `protocolVersion: 2`, and v1 stays the production default. The official registry lists **44 agents** (2026-10-25). Native ACP agents include Gemini CLI (`--acp`), OpenCode (`opencode acp`), Cursor (`cursor-agent acp`), Copilot CLI (`copilot --acp`, public preview), Goose, Kimi, Junie, Qwen Code, Cline, Augment (`auggie --acp`) and Factory Droid. Claude and Codex run through official adapters under the ACP org (`@agentclientprotocol/claude-agent-acp@0.95.0`, `@agentclientprotocol/codex-acp@2.2.2`). Pi runs through the third-party `pi-acp@0.0.34`. [V-src registry@ee673c9]
2. **Since mid-2026, stable ACP covers most of what Codly needs:** `usage_update` (context used/size plus optional cumulative `cost`), `session/list`, `session/resume`, `session/close`, `session/delete`, `logout`, config options (model or mode selectors), `session_info_update`, `notice`, `compaction_*`, `$/cancel_request` and elicitation. **Still unstable:** per-turn token `usage` on the `session/prompt` result, **subagents** (`subagent_update`, `session_message`), `session/fork`, plan operations and LLM providers. [V-src schema v1.26.0]
3. **Both ACP probes ran end to end and are captured** (Gemini 0.62.0 and OpenCode 1.18.31). OpenCode's ACP output is excellent: model and mode `configOptions`, `usage_update` with `cost`, `usage` on the prompt result, and list/resume/close/fork. Gemini's ACP output is good but uses legacy or non-standard fields: an unstable `models` block instead of configOptions, usage in `_meta.quota`, and no sessionCapabilities. **For Gemini, ACP is strictly richer than `--output-format stream-json`.** ACP adds thoughts, diffs, interactive permissions and modes. [V-local]
4. **Gotcha found live:** if the client advertises `fs.readTextFile`, Gemini sends reads of *not-yet-existing* files to the client. My probe returned a generic `-32603` error with an ENOENT message, and Gemini's `write_file` then **failed twice** until the model worked around it with `touch`. Return **`-32002` "Resource not found"** (the spec code) for missing files. Better: **don't advertise `fs` at all** (see Recommendations). [V-local `acp-gemini-write.jsonl`, V-src gemini bundle]
5. **The Rust SDK `agent-client-protocol` is at 3.5.0 (2026-10-22). Its API churns a lot:** 85 releases, majors 1.0 → 2.0 → 3.0 between June and October 2026. The old `Client` trait and `ClientSideConnection` were removed in 0.11 and replaced by builders. It is runtime-neutral (futures, async-io and async-process; no tokio requirement). **The `agent-client-protocol-schema` crate (1.12.0) is semver-stable with additive minors.** Recommendation: use the schema crate's types plus a thin tokio NDJSON JSON-RPC transport inside Codly. Treat the SDK as a reference, not a dependency. [V-src rust-sdk@5ef0fa3, crates.io API]
6. **What ACP loses compared with native protocols:** subscription quota and rate-limit windows (Codex rate limits, Copilot `quotaSnapshots`, Claude plan limits), Claude hooks and the full SDK control protocol, per-turn cost on most agents, and Copilot's ~150 typed session events. Subagents are lost unless both sides opt in to the unstable RFD. The npx adapters also need a **Node runtime**. **Recommendation:** use ACP as the universal path for Gemini, OpenCode, Cursor, Copilot, Goose, Pi and others. Use native adapters for Claude Code (stream-json control protocol) and Codex (app-server). Keep ACP adapters as a fallback "compat mode" for those two.
7. **Cheap auth detection exists for most agents:** `claude auth status --json` [V-local], `codex login status` (exit 0) [V-local], `agent status --format json` (Cursor) [V-docs], `opencode providers list` / `GET /provider/auth` [V-local]. Gemini and Copilot have no status command, so read their config files and env, or run an ACP handshake probe (`initialize` then `session/new`). That probe costs **zero model tokens** and takes about 1.4 s for Gemini or OpenCode [V-local]. Any `session/new` failure with `-32000` means not authenticated.

---

## 1. ACP: the protocol today

### 1.1 Where it lives, versions, governance

| Item | Value | Evidence |
|---|---|---|
| Spec repo | `github.com/agentclientprotocol/agent-client-protocol` (moved from `zed-industries`; 4.5k stars) | [V-src @66ff807, 2026-10-25] |
| Site | https://agentclientprotocol.com (docs source is in the repo under `docs/`) | [V-src] |
| Stable schema | `schema/v1/schema.json`, release **schema-v1.26.0** (2026-10-16) | [V-src] |
| Wire version | `protocolVersion: 1` (an integer) | [V-local] (both agents echoed `1`) |
| v2 | Draft since 2026-07-20 (`docs/announcements/acp-v2-draft.mdx`). Gate it behind version negotiation and feature flags, and keep v1 alongside | [V-docs] |
| Governance | RFD process (`docs/rfds/*.mdx`); lead maintainer Ben Brandt (Zed) | [V-docs] |
| SDKs | TS `@agentclientprotocol/sdk` 2.0.2, Rust `agent-client-protocol` 3.5.0, plus Python, Kotlin and Java | [V-src npm/crates] |
| Registry | `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json` (refreshed hourly; CI checks that each agent returns valid `authMethods`) | [V-src registry README] |
| TCK | `agentclientprotocol/acp-tck` (test compatibility kit) | [V-src gh] |

The main v2 changes, which Codly's internal model should already support:

- Updates may arrive outside a turn.
- `session/prompt` returns the inserted user `messageId`, not the end of the turn. An explicit idle state signals readiness.
- Messages, tool calls and terminal output use upsert-by-ID semantics: omitted means unchanged, `null` clears, chunks append.
- Diffs become structured file changes (add, delete, modify, move, copy, binary) with an optional `git_patch`.
- Permission requests carry their own `title` and `subject`.
- Enums accept `_`-prefixed extension values. [V-docs acp-v2-draft.mdx]

### 1.2 Transport

- JSON-RPC 2.0 over the agent's **stdin/stdout**, as **newline-delimited JSON** (one message per line). stderr carries free-form logs. The client spawns the agent as a subprocess. [V-local, V-docs `protocol/v1/transports.mdx`]
- Both sides send requests: the agent calls back into the client for permissions, fs and terminals. The client must keep a pending-request table **and** a dispatcher for incoming requests.
- Copilot also supports `copilot --acp --port 3000` (TCP, NDJSON) [V-docs]. A Streamable HTTP/WebSocket transport RFD exists (`rfds/streamable-http-websocket-transport.mdx`), and the Rust `agent-client-protocol-http` crate implements HTTP/SSE and WebSocket [V-src]. For Codly, stdio is the right default.

### 1.3 Method inventory (stable v1, `schema/v1/meta.json`) [V-src]

| Direction | Method | Notes |
|---|---|---|
| C→A | `initialize` | Negotiates `protocolVersion`, capabilities, `authMethods`, `agentInfo` |
| C→A | `authenticate` | `{methodId}`. Only needed when `session/new` returns `-32000` |
| C→A | `logout` | Stabilized |
| C→A | `session/new` | `{cwd (absolute), mcpServers: [] (required), additionalDirectories?}` → `{sessionId, modes?, configOptions?}` |
| C→A | `session/load` | Needs `agentCapabilities.loadSession`. Replays history as `session/update`s |
| C→A | `session/resume` | Needs `sessionCapabilities.resume`. Reattaches without replay |
| C→A | `session/list` | Needs `sessionCapabilities.list`. `{cwd?, cursor?}` → `{sessions:[{sessionId,cwd,title,updatedAt}], nextCursor?}` |
| C→A | `session/close`, `session/delete` | Capability-gated |
| C→A | `session/prompt` | `{sessionId, prompt: ContentBlock[]}` → `{stopReason, _meta?}`. The response arrives **after** the whole turn |
| C→A | `session/cancel` | A **notification** (no id). The agent ends the pending prompt with `stopReason: "cancelled"` |
| C→A | `session/set_mode` | `{sessionId, modeId}` |
| C→A | `session/set_config_option` | `{sessionId, configId, value}`. **This replaced `session/set_model`**: the unstable session-model API was removed (#1325). Gemini 0.62 still uses the legacy one (§4.4) |
| A→C | `session/update` | Notification. The main event stream (§1.6) |
| A→C | `session/request_permission` | Request. The client **must** answer or the agent blocks |
| A→C | `fs/read_text_file`, `fs/write_text_file` | Only if the client advertised `fs.*` |
| A→C | `terminal/create`, `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, `terminal/release` | Only if the client advertised `terminal: true` |
| A→C | `elicitation/create`, `elicitation/complete` | Structured user questions (stabilized) |
| both | `$/cancel_request` | Generic request cancellation (stabilized) |

The unstable set (`meta.unstable.json`) adds `session/fork`, `providers/list|set|disable`, and `mcp/message` (MCP over ACP). Extension methods start with `_` (for example `_auth/status_update` from the Claude adapter). Vendor methods are namespaced (for example `cursor/ask_question`). [V-src]

### 1.4 Real lifecycle (from `fixtures/acp-opencode.jsonl`) [V-local]

```text
c→a  {"id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":false},"clientInfo":{"name":"codly-acp-probe","title":"Codly ACP probe","version":"0.0.1"}}}
a→c  {"id":0,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"mcpCapabilities":{"http":true,"sse":true},"promptCapabilities":{"embeddedContext":true,"image":true},"sessionCapabilities":{"close":{},"fork":{},"list":{},"resume":{}}},"authMethods":[{"description":"Run `opencode auth login` in the terminal","name":"Login with opencode","id":"opencode-login"}],"agentInfo":{"name":"OpenCode","version":"1.18.31"}}}
c→a  {"id":1,"method":"session/new","params":{"cwd":"/…/fixture-repo2","mcpServers":[]}}
a→c  {"id":1,"result":{"sessionId":"ses_e8afc29c…","configOptions":[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"opencode/big-pickle","options":[…621 items…]},{"id":"mode","name":"Session Mode","category":"mode","type":"select","currentValue":"build","options":[{"value":"build",…},{"value":"plan",…}]}]}}
c→a  {"id":2,"method":"session/prompt","params":{"sessionId":"ses_…","prompt":[{"type":"text","text":"Read README.md and reply DONE"}]}}
a→c  session/update available_commands_update   (slash commands, incl. the user's ~/.claude skills!)
a→c  session/update tool_call        {"toolCallId":"ccd5…","title":"read","kind":"read","status":"pending","locations":[],"rawInput":{}}
a→c  session/update tool_call_update {"status":"in_progress","locations":[{"path":"/…/README.md"}],"rawInput":{"filePath":"/…/README.md"}}
a→c  session/update tool_call_update {"status":"completed","title":"README.md","content":[{"type":"content","content":{"type":"text","text":"# Fixture Repo…"}}],"rawOutput":{"output":"<path>…","metadata":{…}}}
a→c  session/update agent_message_chunk {"messageId":"msg_1750…","content":{"type":"text","text":"DONE"}}
a→c  session/update usage_update {"used":15040,"size":200000,"cost":{"amount":0,"currency":"USD"}}
a→c  {"id":2,"result":{"stopReason":"end_turn","usage":{"inputTokens":14970,"outputTokens":8,"totalTokens":15050,"thoughtTokens":2,"cachedReadTokens":70},"_meta":{}}}
c→a  {"id":3,"method":"session/list","params":{"cwd":"/…/fixture-repo2"}}
a→c  {"id":3,"result":{"sessions":[{"sessionId":"ses_…","cwd":"/…","title":"New session - 2026-10-25T21:42:28.144Z","updatedAt":"…"}, …]}}
```

### 1.5 Capabilities (stable v1) [V-src]

- `ClientCapabilities`: `fs.readTextFile`, `fs.writeTextFile`, `terminal` (bool), `session` (client session capabilities), `auth`, `elicitation`, `_meta`. With the unstable `subagents` RFD, also `subagents: {}`.
- `AgentCapabilities`: `loadSession`, `promptCapabilities{image,audio,embeddedContext}`, `mcpCapabilities{http,sse}` (stdio MCP is mandatory), `sessionCapabilities{list,delete,additionalDirectories,resume,close}`, `auth`, `_meta`.
- `AuthMethod` is `{id,name,description?}` (type `agent`, the default: the agent runs its own OAuth or browser flow when the client calls `authenticate`) or `{type:"terminal", args[], env{}}`. With `terminal`, the client relaunches the agent binary with those args in a TTY for an interactive login. The registry only lists agents that support one of these two. [V-src registry AUTHENTICATION.md]

### 1.6 `session/update` variants

Stable v1 (`SessionUpdate.oneOf`, tag field `sessionUpdate`): [V-src]

| `sessionUpdate` | Payload (beyond `sessionUpdate`) |
|---|---|
| `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk` | `content: ContentBlock`, `messageId?` (OpenCode sends it; Gemini does not) |
| `tool_call` | `ToolCall` (below) |
| `tool_call_update` | `ToolCallUpdate` (every field except `toolCallId` is optional; `content` and `locations` **replace**) |
| `plan` | `entries: [{content, priority: high\|medium\|low, status: pending\|in_progress\|completed}]` (the full list each time) |
| `available_commands_update` | `availableCommands: [{name, description, input?: {hint}}]` |
| `current_mode_update` | `currentModeId` |
| `config_option_update` | `configOptions` |
| `session_info_update` | `title?`, `updatedAt?` |
| `usage_update` | `used` (tokens in context), `size` (window), `cost?: {amount, currency}` (cumulative) |
| `notice` | Non-history info for the user (stabilized 1.25.0) |
| `compaction_update`, `compaction_summary_chunk` | Context compaction lifecycle (stabilized 1.25.0) |

Unstable additions: `plan_update`, `plan_removed`, `subagent_update`, `session_message`, `session_message_chunk`. The unstable `PromptResponse.usage` is `{totalTokens, inputTokens, outputTokens, thoughtTokens?, cachedReadTokens?, cachedWriteTokens?}`. OpenCode already sends it [V-local].

**ToolCall** fields: `toolCallId`, `title`, `name?` (stabilized 1.22.0: the programmatic tool name), `kind`, `status`, `content[]`, `locations[]`, `rawInput?`, `rawOutput?`, `_meta?`.

- `kind` ∈ `read, edit, delete, move, search, execute, think, fetch, switch_mode, other`.
- `status` ∈ `pending, in_progress, completed, failed`.
- `content[]` items are `{type:"content", content: ContentBlock}`, `{type:"diff", path, oldText|null, newText}` or `{type:"terminal", terminalId}`.
- `locations[]` items are `{path (absolute), line?}`.

These map directly onto Living-World animations.

**Observed differences between agents** [V-local]:

| | Gemini 0.62 | OpenCode 1.18.31 |
|---|---|---|
| Tool IDs | `read_file__read_file_1792964533840_0` | UUIDs |
| First status | `in_progress` (or `pending` when approval is needed) | `pending` with empty `rawInput`, then `in_progress` with the input |
| `rawInput`/`rawOutput` | absent | present (rich, including `metadata.display`) |
| Diffs | `{type:"diff", oldText:"", newText:"hi", _meta:{kind:"add"}}` on pending and completed | `write` returned only text ("Wrote file successfully."), with no diff content |
| Read output | `content: []` (nothing) | file text in `content` |
| Shell output | `content: []` on completion (output not exposed) | output text in `content` and `rawOutput` |
| Extra pseudo-tool | `update_topic` (`kind:"think"`), the model's "strategic intent" | none |
| `messageId` on chunks | no | yes |

**Implication:** Codly cannot rely on `content` alone. For files touched, use `locations` plus Codly's own fs watcher and `git diff` on the worktree. For terminal output, use the `_meta` terminal extension (§1.9) or Codly-owned terminals.

### 1.7 Permissions [V-local `acp-gemini-write.jsonl`]

```json
{"jsonrpc":"2.0","id":6,"method":"session/request_permission","params":{
  "sessionId":"dada2c35-…",
  "options":[{"optionId":"proceed_always","name":"Allow for this session","kind":"allow_always"},
             {"optionId":"proceed_once","name":"Allow","kind":"allow_once"},
             {"optionId":"cancel","name":"Reject","kind":"reject_once"}],
  "toolCall":{"toolCallId":"write_file__write_file_1792964628130_0","status":"pending","title":"Writing to gemini.txt",
              "content":[{"type":"diff","path":"/…/gemini.txt","oldText":"","newText":"hi","_meta":{"kind":"add"}}],
              "locations":[{"path":"/…/gemini.txt"}],"kind":"edit"}}}
```

The client answers with `{"outcome":{"outcome":"selected","optionId":"proceed_once"}}`, or `{"outcome":{"outcome":"cancelled"}}` when the turn was cancelled. **Choose by `kind`, never by `optionId`**: option IDs are agent-specific. Cursor documents `allow-once`/`allow-always`/`reject-once` [V-docs], and Gemini uses `proceed_*`. In my runs, Gemini asked for `write_file` and `run_shell_command(touch …)`. It ran `git status --short` and reads **without asking** (built-in read-only allowlist). OpenCode asked **nothing**, because its default permissions allow edit and bash [V-local]. Codly therefore cannot depend on permission requests for safety. Use worktree isolation, set the agent's own mode to a stricter one (Gemini `default`/`plan`; OpenCode `plan` mode, or `permission` config via `OPENCODE_CONFIG_CONTENT`), and run the fs watcher.

### 1.8 Client-side fs and terminal methods

- `fs/read_text_file {sessionId, path, line?, limit?}` → `{content}`. `fs/write_text_file {sessionId, path, content}` → `{}`. The spec says the client **MUST** create the file if it doesn't exist. [V-src]
- **Gemini behavior** [V-local + V-src `acpFileSystemService.ts` in the 0.62 bundle]: Gemini only delegates paths **inside the session root and outside `~/.gemini`**. The `read_file` tool did *not* delegate in my run, but `write_file` reads the old content through the client first. It then turns errors whose message contains `Resource not found`, `ENOENT`, `does not exist` or `No such file` into `code: "ENOENT"`. My `-32603` "ENOENT: …" reply still produced `Error checking existing file` twice. **Fix: reply `{"code":-32002,"message":"Resource not found: <path>"}`** (ACP `ErrorCode` −32002). [Unverified for the fixed reply: paid-run budget exhausted]
- OpenCode never called `fs/*` even though fs was advertised. It reads and writes disk directly [V-local].
- Terminals: `terminal/create {sessionId, command, args[], env[{name,value}], cwd?, outputByteLimit?}` → `{terminalId}`; `terminal/output` → `{output, truncated, exitStatus?}`; `terminal/wait_for_exit` → `{exitCode|null, signal|null}`; plus `kill` and `release`. Agents embed a terminal with a `{type:"terminal", terminalId}` tool content. [V-src]

### 1.9 De facto extensions worth supporting

- **Terminal output in `_meta`** (Zed-originated). The Claude adapter and codex-acp read it. The client opts in with `clientCapabilities._meta = {"terminal_output": true}` or `{"terminal_output_delta": true}`. Tool calls then carry `_meta.terminal_info {terminal_id}`, `_meta.terminal_output {terminal_id, data}` (or `terminal_output_delta`) and `_meta.terminal_exit {terminal_id, exit_code|null, signal|null}`. Codly gets live command output **without** implementing `terminal/*`. [V-src claude-agent-acp `tool-calls/client-capabilities.ts`, `renderer.ts`; codex-acp `AcpV2SessionUpdate.ts`]
- **`_auth/status_update`** notification (Claude adapter): `{authStatus:{kind: account|api_key|gateway|external|none, label, detail?, account?{email,organization,plan}}}`. The adapter advertises it with `agentCapabilities._meta.authStatus`. [V-src `auth-status.ts`]
- **Cursor methods:** `cursor/ask_question` and `cursor/create_plan` are **blocking** requests the client must answer. `cursor/update_todos`, `cursor/task` (subagent completion) and `cursor/generate_image` are notifications. [V-docs cursor.com/docs/cli/acp]
- **Usage `_meta`:** Gemini puts per-turn tokens in `PromptResponse._meta.quota = {token_count:{input_tokens,output_tokens}, model_usage:[{model, token_count}]}` [V-local].

### 1.10 Subagents (unstable RFD, the key for the Living World)

The client sends `clientCapabilities.subagents: {}`. The agent then announces each child with `subagent_update {sessionId (child), title?, description?, capabilities?{cancel}, state?: running|requires_action|idle(+stopReason)}` on the **parent** stream. The child's own updates flow under the child `sessionId`, and `session_message` links parent and child transcripts. `session/cancel` on a child `sessionId` stops only that child. **Implementers today:** claude-agent-acp (Agent/Task subagents, since 0.9x) and codex-acp ("native ACP subagent sessions"). Copilot CLI exposes "subagent IDs" to ACP clients (changelog). [V-src claude-agent-acp README, codex-acp README, schema.unstable.json; V-docs Copilot changelog]

### 1.11 Errors [V-src]

`-32700` parse, `-32600` invalid request, `-32601` method not found, `-32602` invalid params, `-32603` internal, **`-32000` auth required**, **`-32002` resource not found**, `-32800` request cancelled.

---

## 2. Rust: the `agent-client-protocol` crate

### 2.1 Facts [V-src rust-sdk@5ef0fa3 (2026-10-25), crates.io API today]

| Crate | Latest | Notes |
|---|---|---|
| `agent-client-protocol` | **3.5.0** (2026-10-22) | Edition 2024, MSRV 1.88. No default features. `process` = spawn agents (`AcpAgent`), `stdio` = agent side. Unstable features: `unstable_subagents`, `unstable_end_turn_token_usage`, `unstable_session_fork`, `unstable_protocol_v2`, … |
| `agent-client-protocol-schema` | **1.12.0** (2026-10-16) | Generated serde types for v1 (and v2 behind a flag). 1.x since 2026-06-24, monthly additive minors |
| `-conductor`, `-polyfill`, `-http`, `-rmcp` (4.0.5), `-trace-viewer` | 3.5.0 | Proxy chains, HTTP/WS transport, MCP integration |

- **Runtime:** the core is runtime-neutral. It depends on `futures`; the `process` feature adds `async-io` and `async-process`. **tokio is not required** (dev-only), so it runs inside Tauri's tokio runtime without conflict. Connection futures are `Send`: the 0.10-era `LocalSet`/`spawn_local` requirement is gone. [V-src Cargo.toml, `md/migration_v0.11.x.md`]
- **Churn:** 85 versions since 2025-07-24. Breaking majors: 1.0 (2026-06-24), 2.0 (2026-07-23), 3.0 (2026-10-06). The 0.10 → 0.11 change removed `ClientSideConnection::new(handler, out, in, spawn)` and the `Client` trait in favor of `Client.builder()…connect_with(transport, async |cx| …)`. [V-src crates.io]
- **Download stats** (crates.io): about 5.6M total for the crate, about 2.2M recent (driven by Zed and other agent builds). [V-src]

### 2.2 Current API shape (v3.5, stable-v1 path) [V-src `examples/yolo_one_shot_client.rs`]

```rust
use agent_client_protocol::{AcpAgent, Agent, Client, ConnectionTo};
use agent_client_protocol::schema::{ProtocolVersion, v1::*};

let agent = AcpAgent::from_str("gemini --acp")?            // or AcpAgent::new(AcpAgentConfig::new("gemini").arg("--acp").env("K","V"))
    .with_debug(|line, dir| tracing::trace!(?dir, line));   // raw line tap (stdin/stdout/stderr)

Client.builder()
    .name("codly")
    .on_receive_notification(async move |n: SessionNotification, _cx| { /* n.update */ Ok(()) },
                             agent_client_protocol::on_receive_notification!())
    .on_receive_request(async move |req: RequestPermissionRequest, responder, _cx| {
        responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
    }, agent_client_protocol::on_receive_request!())
    .connect_with(agent, |cx: ConnectionTo<Agent>| async move {
        let init = cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
        let s = cx.send_request(NewSessionRequest::new(cwd)).block_task().await?;
        let r = cx.send_request(PromptRequest::new(s.session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new("hi"))])).block_task().await?;
        Ok(())
    }).await?;
```

Gotchas from the rustdoc [V-src `jsonrpc.rs`]:

- Awaiting `block_task()` **inside an incoming handler deadlocks**. Spawn the work, or use `.detach()`.
- Dropping a pending response future sends a cancel to the peer.
- `AcpAgent` connections install a guard that kills the agent's **process group** on Unix.
- There are presets `AcpAgent::claude_agent()` and `AcpAgent::codex()`, which run `npx -y …@latest`. Do not ship those as-is (unpinned npx).

---

## 3. Who speaks ACP (registry snapshot 2026-10-25, commit ee673c9) [V-src]

All 44 entries ship an `agent.json`. The table shows the darwin-arm64 launch command.

| Agent | Version | Launch | Type |
|---|---|---|---|
| **Gemini CLI** | 0.65.0 | `npx @google/gemini-cli@0.65.0 --acp` | native (`--experimental-acp` is deprecated, see §4) |
| **OpenCode** | 1.19.4 | `./opencode acp` (GitHub release zip from `anomalyco/opencode`) | native |
| OpenCode V2 | 2.1.4 | `./opencode acp` (opencode.ai/files/bin/2.1.4) | native, separate v2 line |
| **Cursor** | 2026.10.22 | `./dist-package/cursor-agent acp` (downloads.cursor.com tarball) | native (hidden command) |
| **Copilot CLI** | 1.0.103 | `npx @github/copilot@1.0.103 --acp` | native, "public preview" |
| Copilot (language server) | 1.561.0 | `npx @github/copilot-language-server@1.561.0 --acp` | native |
| **Claude Agent** | 0.95.0 | `npx @agentclientprotocol/claude-agent-acp@0.95.0` | adapter (Claude Agent SDK 0.3.309) |
| **Codex** | 2.2.2 | `npx @agentclientprotocol/codex-acp@2.2.2` | adapter (TS; spawns `codex app-server`; bundles `@openai/codex` ^0.164) |
| **pi ACP** | 0.0.34 | `npx pi-acp@0.0.34` | 3rd-party adapter → `pi --mode rpc` |
| Goose | 1.56.0 | `./goose acp` | native |
| Auggie (Augment) | 0.36.0 | `npx @augmentcode/auggie@0.36.0 --acp` | native |
| Cline | 3.0.73 | `npx cline@3.0.73 --acp` | native |
| Factory Droid | 0.249.0 | `npx droid@0.249.0 exec --output-format acp-daemon` | native |
| Qwen Code | 0.26.1 | `npx @qwen-code/qwen-code@0.26.1 --acp --experimental-skills` | native |
| Kimi CLI | 1.52.0 | `./kimi acp` | native |
| Junie (JetBrains) | 3813.9.0 | `junie --acp=true` | native |
| Mistral Vibe | 3.0.1 | `./vibe-acp` | native |
| Kilo | 7.8.17 | `kilo acp` / `npx @kilocode/cli@7.8.17 acp` | native |
| Devin | 3000.11.205 | `./bin/devin acp` | native |
| Grok Build | 1.0.70 | `npx @xai-official/grok@1.0.70 agent stdio` | native |
| Google Antigravity | 1.4.3 | `./agy_acp_server.par` | native |
| Others | | amp-acp, autohand, bob (IBM), codebuddy-code, cortex-code (Snowflake), corust, crow-cli, deepagents (`deepagents-acp`), dimcode, dirac, fast-agent, glm-acp-agent, harn, kimchi, minimax-code, minion-code, nova, poolside, qoder, sigit, stakpak, vtcode, agoragentic | |

Quarantined (currently failing CI): `agoragentic-acp`, `crow-cli`, `deepagents` ("Missing npm dependency"), `minion-code`, `qoder`, `vtcode`, `fast-agent` (`quarantine.json`). The ACP site's agents page lists more (Kiro CLI, OpenHands, Hermes, Docker cagent, Blackbox, …). It still links Claude to the old `zed-industries/claude-agent-acp` repo. **Stale names:** `@zed-industries/claude-code-acp` and `@zed-industries/claude-agent-acp` are deprecated on npm (renamed to `@agentclientprotocol/claude-agent-acp`), and `@zed-industries/codex-acp` is deprecated in favor of `@agentclientprotocol/codex-acp`. [V-src npm view]

---

## 4. Gemini CLI (installed 0.62.0; latest 0.65.0)

### 4.1 Relevant flags (`gemini --help`, 0.62.0) [V-local]

| Flag | Meaning |
|---|---|
| `-p, --prompt <text>` | Headless. Appended to stdin if any. A bare positional now means **interactive** (there's a warning) |
| `-o, --output-format text\|json\|stream-json` | Output format |
| `-m, --model <id>` | e.g. `gemini-2.5-flash`. The 0.62 ACP model list offers `auto`, `gemini-3.1-pro-preview(-customtools)`, `gemini-3-flash-preview`, `gemini-2.5-pro`, `gemini-3.8-flash`, `gemini-3.5-flash-lite` |
| `--approval-mode default\|auto_edit\|yolo\|plan` | `plan` is read-only |
| `-y, --yolo` | Same as `--approval-mode yolo` |
| `--policy <files>` / `--admin-policy` | Policy engine (replaces the deprecated `--allowed-tools`) |
| `--acp` | **ACP mode (stable flag).** `--experimental-acp` still works but is "deprecated, use --acp" |
| `-r, --resume latest\|<index>` | Resume a session; also `--list-sessions`, `--delete-session` |
| `--session-id <uuid>` | **Start a session with a caller-chosen UUID.** Codly can pre-assign IDs |
| `--session-file <json>` | Load a session from a file |
| `-w, --worktree [name]` | Built-in git worktree start |
| `--skip-trust` | Trust the cwd for this run (needed in fresh dirs) |
| `--include-directories`, `-e/--extensions`, `--allowed-mcp-server-names`, `-s/--sandbox`, `--raw-output` | |

### 4.2 Headless `stream-json` schema [V-local `gemini-stream*.jsonl`, V-src bundle `JsonStreamEventType`]

The event `type` ∈ `init | message | tool_use | tool_result | error | result`:

```jsonl
{"type":"init","timestamp":"2026-10-25T21:37:16.926Z","session_id":"54576833-…","model":"gemini-2.5-flash"}
{"type":"message","timestamp":"…","role":"user","content":"Reply with exactly: pong"}
{"type":"tool_use","timestamp":"…","tool_name":"read_file","tool_id":"read_file__read_file_1792964578162_1","parameters":{"file_path":"README.md"}}
{"type":"tool_result","timestamp":"…","tool_id":"read_file__read_file_1792964578162_1","status":"success","output":""}
{"type":"message","timestamp":"…","role":"assistant","content":"pong","delta":true}
{"type":"result","timestamp":"…","status":"success","stats":{"total_tokens":9557,"input_tokens":9533,"output_tokens":0,"cached":0,"input":9533,"duration_ms":1608,"tool_calls":0,"models":{"gemini-2.5-flash":{…same keys…}}}}
```

- Error shapes from source: `{"type":"error","severity":"error"|"warning","message":…}`, and on failure `{"type":"result","status":"error","error":{"type":"FatalToolExecutionError"|…,"message":…},"stats":{…}}` followed by a non-zero exit.
- `tool_result` on error carries `error:{type,message}`.
- **Quirks:**
  - `output_tokens: 0` while the model clearly answered: `total_tokens` includes thinking tokens and there is no thought field.
  - **No thoughts, no diffs, no cost, no permission dialog.** Approvals must be decided up front with `--approval-mode` or a policy.
  - `tool_result.output` for `read_file` is `""`.
  - The `update_topic` tool appears as a normal tool.
  - stderr prints a 256-color warning. Ignore stderr.
- Latency: about 3.3 s wall for "pong" (CLI start plus model).

### 4.3 Sessions

Every headless and ACP prompt run is persisted per project (`gemini --list-sessions` showed all four runs). The **ACP `sessionId` equals the Gemini session UUID**, so a session started over ACP can be resumed with `gemini --resume <n>` or `session/load` [V-local].

### 4.4 ACP specifics (0.62.0) [V-local `acp-gemini.jsonl`, `acp-gemini-write.jsonl`]

- `initialize` → `agentCapabilities {loadSession:true, promptCapabilities{image,audio,embeddedContext}, mcpCapabilities{http,sse}}`. There are **no `sessionCapabilities`**, so no list, resume or close over ACP. `agentInfo {name:"gemini-cli", version:"0.62.0"}`.
- `authMethods`: `oauth-personal` (Google login), `gemini-api-key` (`_meta.api-key.provider:"google"`), `vertex-ai`, `gateway` (`_meta.gateway{protocol:"google"}`).
- `session/new` → `modes {currentModeId:"default", availableModes:[default, autoEdit, yolo, plan]}`, plus a **legacy `models {currentModelId, availableModels:[{modelId,name,description?}]}`**. Change the model with the legacy `session/set_model` (Gemini implements `unstable_setSessionModel`) [V-src bundle]. Change the mode with `session/set_mode`. There are no `configOptions`.
- Stream: `available_commands_update` (20 commands: memory, …), `agent_thought_chunk` (bold-titled summaries such as `**Initiating README Analysis**\n…`), `tool_call`/`tool_call_update` (§1.6), `agent_message_chunk`. The result is `{stopReason:"end_turn", _meta:{quota:{token_count:{input_tokens:26988,output_tokens:19}, model_usage:[…]}}}`. There is **no `usage_update` and no cost**.
- Timings: initialize 1.27 s, session/new 0.1 s, full read-and-reply turn 2.0 s.
- Auth state on this Mac: `~/.gemini/settings.json` has `security.auth.selectedType: "gemini-api-key"`. `GEMINI_API_KEY` is unset, so the key comes from the OS keychain service `gemini-cli-api-key` (HybridTokenStorage; OAuth uses `gemini-cli-oauth`; the file fallback is `~/.gemini/oauth_creds.json`). [V-src]. **Codly must never read those.** Detect only `selectedType` and env presence, or run the ACP probe.

---

## 5. OpenCode (installed 1.18.31; latest 1.19.4; repo moved to `anomalyco/opencode`)

### 5.1 Flags [V-local `opencode --help`, `opencode run --help`]

- `opencode run [message..]`:
  - `--format default|json`, `-m provider/model`, `--agent`, `-c/--continue`, `-s/--session <id>`, `--fork`, `-f/--file`, `--title`, `--dir`, `--variant` (reasoning effort), `--thinking`.
  - `--auto` (auto-approve permissions that are not explicitly denied).
  - `--attach http://host:port` (run against a running server), `-p/-u` basic auth (`OPENCODE_SERVER_PASSWORD`/`OPENCODE_SERVER_USERNAME`), `--port`, `--pure` (no external plugins).
- `opencode serve [--port N --hostname 127.0.0.1 --cors … --mdns]`, `opencode acp [--cwd DIR --port …]`, `opencode models [provider]`, `opencode providers list|login|logout` (alias `auth`), `opencode session …`, `opencode export|import`, `opencode stats`.
- Model pinning without touching user config: `OPENCODE_CONFIG_CONTENT='{"model":"opencode/big-pickle"}'` [V-local]. Free Zen models (`opencode/big-pickle`, `opencode/*-free`) work with **0 stored credentials** (`cost: 0`) [V-local]. That makes OpenCode a good zero-cost backend for Codly's own CI and tests.

### 5.2 `opencode run --format json` [V-local `opencode-run*.jsonl`, V-src `packages/opencode/src/cli/cmd/run.ts@v1.19.4`]

Each line is `{type, timestamp (epoch ms), sessionID, ...data}`, where `type` ∈ `step_start | text | tool_use | step_finish | reasoning (only with --thinking) | error`:

```jsonl
{"type":"step_start","timestamp":1792964595049,"sessionID":"ses_…","part":{"id":"prt_…","messageID":"msg_…","sessionID":"ses_…","snapshot":"559dae57…","type":"step-start"}}
{"type":"tool_use","timestamp":…,"sessionID":"ses_…","part":{"type":"tool","tool":"read","callID":"6b6b…","state":{"status":"completed","input":{"filePath":"/…/README.md"},"output":"<path>…</path>\n<type>file</type>\n<content>…","metadata":{…},"title":"README.md","time":{"start":…,"end":…}}}}
{"type":"step_finish","timestamp":…,"part":{"reason":"tool-calls","snapshot":"…","type":"step-finish","tokens":{"total":15398,"input":14914,"output":74,"reasoning":340,"cache":{"write":0,"read":70}},"cost":0}}
{"type":"text","timestamp":…,"part":{"type":"text","text":"DONE","time":{"start":…,"end":…}}}
{"type":"step_finish","timestamp":…,"part":{"reason":"stop", "tokens":{…},"cost":0}}
```

- `tool_use` is emitted **only when a tool completes or errors** (no start event), and `text` only when the part ends. `run --format json` is therefore not a live stream. Use `serve` + SSE or ACP for live animation.
- **Permissions:** without `--auto`, `run` **auto-rejects** every `permission.asked` (source). With `--auto` it replies `once`.
- Per-step `tokens` and `cost` are in `step_finish` (the best native cost data among these CLIs).
- Observed once: the first `opencode run --format json` in a fresh dir **hung for 120 s with no stdout**, although the session was created and titled. Re-running worked in 4 s. [V-local, root cause unknown]. Add a watchdog.

### 5.3 `opencode serve`: HTTP + SSE [V-local: started on 127.0.0.1:4199, fetched `/doc`]

- OpenAPI **3.1** at `GET /doc` (479 KB, ~170 operations). Prints `opencode server listening on http://127.0.0.1:PORT`. Warns that it is unsecured unless `OPENCODE_SERVER_PASSWORD` is set (basic auth).
- Key endpoints:
  - Sessions: `GET/POST /session`, `GET /session/{id}/message`, `POST /session/{id}/message` (sync prompt), `POST /session/{id}/prompt_async`, `POST /session/{id}/abort`, `POST /session/{id}/fork`, `GET /session/{id}/children` (subagents), `GET /session/{id}/diff`, `GET /session/{id}/todo`, `POST /session/{id}/revert`.
  - Permissions and questions: `GET /permission`, `POST /permission/{requestID}/reply`, `GET /question`, `POST /question/{id}/reply`.
  - Other: `GET /provider`, `GET /provider/auth`, `GET /config/providers`, `GET /vcs/diff`, `GET /file/status`, `GET/POST /pty` (PTY sessions over WebSocket), `GET/POST /experimental/worktree`.
  - A newer `/api/session/...` surface: `/api/session/{id}/event` (per-session SSE), `/wait`, `/interrupt`, `/model`.
- SSE: `GET /event` (instance-scoped) and `GET /global/event`. The first record is `data: {"id":"evt_…","type":"server.connected","properties":{}}` [V-local]. **89 event types** are in the schema, including `session.created/updated/deleted/status/idle/error/diff/compacted`, `message.updated`, `message.part.updated` (`{sessionID, part: Part, time}`), `message.part.delta`, `permission.asked/replied` (`{id:"per_…", sessionID, permission, patterns[], metadata, always[], tool}`), `question.*`, `todo.updated`, `file.edited`, `pty.*`, `worktree.ready/failed`, `vcs.branch.updated`, and the new granular `session.next.*` stream (`text.delta`, `reasoning.delta`, `tool.input.delta`, `tool.called/success/failed`, `step.started/ended`, `shell.started/ended`, …).
- `Part` types: `text, subtask, reasoning, file, tool, step-start, step-finish, snapshot, patch, agent, retry, compaction`. `ToolState` is `pending|running|completed|error`. `AssistantMessage` has `cost`, `tokens`, `modelID`, `providerID`, `agent` and `parentID`.
- JS SDK: `@opencode-ai/sdk@1.19.4`, with `createOpencode()` / `createOpencodeServer({hostname,port})` / `createOpencodeClient({baseUrl})`. The server helper spawns `opencode serve --hostname=… --port=…` and parses the "listening" line. [V-src `packages/sdk/js/src/server.ts@v1.19.4`]. Codly can do the same from Rust with `reqwest` + `eventsource-stream`. No JS is needed.

### 5.4 ACP (`opencode acp`) [V-local §1.4]

This is the most complete ACP implementation tested. Notes:

- Model and mode come as `configOptions` with `category: "model"` (621 options) and `category: "mode"` (`build`, `plan`). There is no `modes` block.
- Sessions are shared with the TUI and serve (`ses_*` IDs show in `opencode session list`), so ACP-started sessions can be continued in other surfaces. Every `session/new` creates a persisted session, which means an ACP *auth probe* leaves "New session - …" entries. Follow it with `session/close` or `session/delete`, or use `opencode providers list` for detection.
- `available_commands_update` included the user's **`~/.claude/skills`** (OpenCode loads Claude skills). Expect large command lists (about 80 KB in the fixture).

### 5.5 Auth state

`opencode providers list` printed `0 credentials` in `~/.local/share/opencode/auth.json`, and env providers Bedrock (`AWS_REGION`), OpenRouter (`OPENROUTER_API_KEY`) and OpenAI (`OPENAI_API_KEY`) [V-local]. `GET /provider/auth` lists the login methods per provider, e.g. `openai: ChatGPT Pro/Plus (browser|headless)`, `github-copilot: Login with GitHub Copilot` [V-local]. **OpenCode can itself use ChatGPT and Copilot subscriptions.** [Claim, docs; not exercised]

---

## 6. Cursor CLI (not installed; docs fetched today)

- Binary: **`agent`** (the old name `cursor-agent` is still what the ACP registry runs). Global flags:
  - `-p/--print`, `--output-format text|json|stream-json`, `--stream-partial-output`, `--resume [chatId]`, `--continue`, `--model`, `--mode plan|ask`, `--plan`, `--list-models`.
  - `-f/--force` (alias `--yolo`), `--sandbox enabled|disabled`, `--approve-mcps`, `--trust` (headless only), `--workspace <path>`, `-w/--worktree [name]` (under `~/.cursor/worktrees/<repo>/<name>`), `--worktree-base`.
  - `--api-key` (`CURSOR_API_KEY`), `-H` custom headers.
  - Commands: `login`, `logout`, **`status|whoami --format json`**, `about --format json`, `models`, `mcp …`, `create-chat` (returns a new chat ID; useful to pre-create sessions), `ls`, `resume`, and `acp` (hidden). [V-docs cursor.com/docs/cli/reference/parameters]
- `stream-json` (NDJSON) [V-docs cursor.com/docs/cli/reference/output-format]:
  - `{"type":"system","subtype":"init","apiKeySource":"env|flag|login","cwd","session_id","model","permissionMode"}`
  - `{"type":"user","message":{"role":"user","content":[{"type":"text","text"}]},"session_id"}`
  - `{"type":"assistant","message":{…},"session_id"}`, one per complete message segment. With `--stream-partial-output`, **use only events that have `timestamp_ms` and no `model_call_id`**; the other two kinds are duplicate flushes.
  - `{"type":"tool_call","subtype":"started|completed","call_id","tool_call":{"readToolCall"|"writeToolCall"|"function":{…args, result:{success:{…}}}},"session_id"}`. Write results include `linesCreated` and `fileSize`.
  - `{"type":"result","subtype":"success","duration_ms","duration_api_ms","is_error":false,"result","session_id","request_id?"}`.
  - **No usage or cost field, and thinking is suppressed in print mode.** On failure there is a non-zero exit and possibly no `result`.
- ACP (`agent acp`) [V-docs cursor.com/docs/cli/acp]:
  - Auth method `cursor_login`. Pre-auth works with `agent login`, `--api-key` or `--auth-token`.
  - `session/new` and `session/load`. Modes are `agent`, `plan` and `ask`.
  - Permission options are `allow-once`, `allow-always` and `reject-once`.
  - MCP comes from `.cursor/mcp.json` (team MCP is not supported).
  - Plus the `cursor/*` extension methods (§1.9). An unanswered permission request blocks the tool.

---

## 7. GitHub Copilot CLI (not installed; 1.0.103 per changelog, GA since 2026-02-25)

- Programmatic mode [V-docs docs.github.com cli-programmatic-reference]:
  - `copilot -p "<prompt>"`, `-s/--silent`, **`--output-format json` (JSONL)**, `--model` (`COPILOT_MODEL`), `--agent`.
  - Permissions: `--allow-all`/`--yolo` (= `--allow-all-tools --allow-all-paths --allow-all-urls`; env `COPILOT_ALLOW_ALL=true`), `--allow-tool='shell(git:*)', 'write(README.md)', 'url(github.com)', 'github(create_issue)'`, `--deny-tool`, `--available-tools`, `--excluded-tools`, `--add-dir`.
  - Other: `--no-ask-user`, `--trust-folder` (one-off), `--attachment`, `--share`, `--secret-env-vars`, `--fleet` (parallel subagents; not in ACP).
  - Auth tokens in precedence order: `COPILOT_GITHUB_TOKEN` > `GH_TOKEN` > `GITHUB_TOKEN`. Config lives in `~/.copilot/settings.json` (or `COPILOT_HOME`).
- The JSONL record schema is **not documented** in the reference [V-docs]. The `@github/copilot-sdk` (npm 1.0.26; Rust crate **`github-copilot-sdk` 1.0.26**) drives the CLI over JSON-RPC (`--server`/`--headless`). Its generated `session-events.ts` defines **151 event types**: `session.start|idle|error|usage_info|…`, `assistant.message_delta|message|reasoning_delta|turn_start|turn_end|usage`, `tool.execution_start|progress|partial_result|complete`, `tool.shell_output`, `permission.requested|completed`, `subagent.started|completed|failed`, `elicitation.*`, `exit_plan_mode.*`, and more. Each event is `{id, parentId, timestamp, type, agentId?, ephemeral?, data}`. `assistant.usage.data` includes `model`, `inputTokens`, `outputTokens`, `cacheReadTokens`, `reasoningTokens`, **`cost`**, **`quotaSnapshots`** and `copilotUsage` [V-src github/copilot-sdk main]. I expect `--output-format json` to emit these same records. [Unverified]
- ACP [V-docs docs.github.com acp-server; changelog]:
  - Started with `copilot --acp [--stdio | --port N]`; it is "public preview".
  - Per-server options: `--available-tools`, `--excluded-tools`, `--effort` (`session/new` cannot carry these). `--sandbox/--no-sandbox` is honored.
  - Changelog entries: token usage in prompt results and live `usage_update`, `session/close`, subagent IDs, config options (`allow_all`), BYOK providers in ACP, file diffs for all edit tools, and the `authenticate` call requires a real Copilot login.

---

## 8. Pi coding agent (not installed; source `badlogic/pi-mono@d5b3de2`)

- The package was **renamed** to `@earendil-works/pi-coding-agent` **1.4.0** (binary `pi`). Docs live in `packages/coding-agent/docs/{json,rpc,rpc-commands,sdk}.md`. [V-src]
- `pi --mode json "<prompt>"` emits a session header `{"type":"session","version":3,"id","timestamp","cwd"}`, then events: `agent_start`, `turn_start`, `message_start|message_update|message_end`, `tool_execution_start|update|end` (`{toolCallId, toolName, args, partialResult|result, isError, durationMs}`), `turn_end`, `agent_end {messages, willRetry}`, **`agent_settled {aborted}`** (the true "done"), `queue_update`, `compaction_start/end`, `auto_retry_start/end`, `thinking_level_changed`, `session_info_changed`.
  - `message_update.assistantMessageEvent` ∈ `text_start|text_delta|text_end|thinking_*|toolcall_start|toolcall_delta|toolcall_end`.
  - Top-level `usage` is `{input, output, cacheRead, cacheWrite, totalTokens, cost:{input,output,cacheRead,cacheWrite,total}}`, so **cost in dollars arrives on every update**. [V-src docs/json.md]
- `pi --mode rpc --no-session` is long-lived JSONL. Commands are `{"id","type":"prompt|steer|follow_up|abort|new_session|get_state|get_messages|set_model|get_available_models|set_thinking_level|compact|bash|get_session_stats|fork|clone|switch_session|get_commands|…"}`, and responses are `{"id","type":"response","command","success","data|error"}`. A successful `prompt` response means *accepted*. Wait for `agent_settled`. [V-src docs/rpc.md]
- **Framing gotcha:** split on `\n` only. Node `readline` also splits on U+2028/U+2029 and breaks JSON. Rust `BufRead::lines()` is fine (it splits on `\n` only). [V-src]
- ACP: only through `pi-acp` 0.0.34 (svkozak; "MVP", Zed-centric). It maps messages, tool calls, structured diffs for `edit`, `usage_update` from `get_session_stats.contextUsage`, and `session/load` through `~/.pi/pi-acp/session-map.json`. **Prefer native `--mode rpc` for Pi.** It is a small, clean protocol with cost data.

---

## 9. ACP vs native: what each path gives Codly

| Capability | ACP (v1 stable) | Gemini stream-json | OpenCode serve+SSE | Cursor stream-json | Copilot JSONL/SDK | Pi rpc | Claude native / Codex app-server (other reports) |
|---|---|---|---|---|---|---|---|
| Live text deltas | yes | yes (`delta:true`) | yes (`message.part.delta`) | opt-in partial | yes | yes | yes |
| Thoughts | `agent_thought_chunk` | **no** | yes | **no** | yes | yes | yes |
| Tool start/end | yes | yes | yes | yes | yes | yes | yes |
| Diffs | `diff` content (agent-dependent) | no | `session.diff`, `patch` parts | write args | yes | edit result | yes |
| **Interactive approvals** | **yes** (`request_permission`) | **no** (pre-set mode) | yes (`/permission/{id}/reply`) | **no** (`--force` or not) | yes (SDK) | extension UI | yes |
| Cumulative cost | `usage_update.cost` (OpenCode, Claude adapter; Gemini **no**; Codex **no**) | no | yes (per step) | **no** | yes (`assistant.usage.cost`) | yes | Claude `total_cost_usd`; Codex tokens |
| Subscription quota or rate limits | **no** | no | no | no | `quotaSnapshots` | no | Codex `account/rateLimits/*`; Claude plan via auth status |
| Context window | `usage_update.used/size` | no | tokens | no | yes | `get_session_stats` | yes |
| Resume / list | `session/load`/`resume`/`list` (capability-gated) | `--resume`, `--session-id` | full | `--resume`, `create-chat` | yes | `switch_session` | yes |
| Subagents | unstable RFD (Claude adapter, codex-acp) | no | `/children`, `subtask` parts | `cursor/task` notif | `subagent.*` + `agentId` | n/a | Claude Task events; Codex threads |
| Plans / todos | `plan` | no | `todo.updated` | `cursor/update_todos` | `session.todos_changed` | n/a | yes |
| Model / mode switching mid-session | config options / set_mode | restart | yes | restart | yes | `set_model` | yes |
| Cancel | `session/cancel` | kill | `/abort` | kill | yes | `abort` | yes |
| Steering / queue while busy | no (v2 draft) | no | `prompt_async` | no | yes | `steer`/`follow_up` | partial |

**Features lost by going ACP-only:**

- Subscription quota and rate-limit telemetry (all agents).
- Per-turn cost for Gemini, Cursor and Codex.
- Hooks and permission-mode internals for Claude.
- Steering and queueing, until v2.
- Cursor and Copilot details that live outside ACP (Cursor's `cursor/*` methods are still ACP-carried, so they are kept if Codly implements them).
- Subagent topology, unless Codly opts in to the unstable RFD.

---

## 10. Detecting installed agents and their auth status cheaply

macOS GUI apps launched from Finder **do not inherit the user's shell PATH** (nvm, Homebrew, `~/.local/bin`). Resolve PATH once from a login shell (`$SHELL -lic 'echo $PATH'`; see `fix-path-env-rs` in the Tauri report) and then probe.

| Agent | Binary (probe order) | Version | Auth status (cheap) | Evidence |
|---|---|---|---|---|
| Claude Code | `claude` | `claude --version` → `2.1.309 (Claude Code)` | **`claude auth status --json`** → `{loggedIn, authMethod:"claude.ai", apiProvider, subscriptionType:"team", email, orgId, orgName, …}` (0.18 s, no tokens) | [V-local] |
| Codex | `codex` | `codex --version` → `codex-cli 0.161.0` | **`codex login status`** → "Logged in using ChatGPT", exit 0 | [V-local] |
| Gemini | `gemini` | `gemini --version` → `0.62.0` | No command. Read `~/.gemini/settings.json` `security.auth.selectedType` (`gemini-api-key`/`oauth-personal`/`vertex-ai`), env `GEMINI_API_KEY`/`GOOGLE_API_KEY`/`GOOGLE_GENAI_USE_VERTEXAI`. Definitive: ACP probe (`--acp`, `initialize` + `session/new`, about 1.4 s, 0 tokens; whether an unprompted session is persisted is unverified) | [V-local] |
| OpenCode | `opencode` | `opencode --version` → `1.18.31` | `opencode providers list` (ANSI text; credentials plus env providers) or `GET /provider/auth` on `serve`. Free `opencode/*` models need no auth | [V-local] |
| Cursor | `agent`, then `cursor-agent` | `agent --version` | **`agent status --format json`** (also `about --format json`) | [V-docs] |
| Copilot | `copilot` | `copilot --version` | No status command documented. Check env tokens (`COPILOT_GITHUB_TOKEN`/`GH_TOKEN`/`GITHUB_TOKEN`) and `~/.copilot/` presence, then an ACP probe (`authenticate` requires a real login) | [V-docs] |
| Pi | `pi` | `pi --version` | `~/.pi/agent/auth.json` presence plus provider env keys; RPC `get_available_models` | [V-src docs] |
| Any ACP agent | registry `distribution` | `agentInfo.version` from `initialize` | `session/new` → `-32000` means not authed. Then show `authMethods` (type `terminal`: open a Codly PTY with `args`; type `agent`: call `authenticate`) | [V-src spec] |

Never read keychain items or token files' contents: Gemini `gemini-cli-api-key`/`gemini-cli-oauth`, `~/.local/share/opencode/auth.json`, `~/.pi/agent/auth.json`. Check existence or metadata only, or let the CLI answer.

---

## 11. Recommendations for Codly

**R1. Run two adapter families behind one internal event model.**

- **`AcpAdapter`** (generic, one Rust implementation) is the **primary path for Gemini CLI, OpenCode, Cursor, Copilot CLI, Goose, Qwen, Kimi, Auggie, Junie** and any registry agent.
- **Native adapters** are the **primary path for Claude Code** (`claude -p --input-format stream-json --output-format stream-json` + control protocol) **and Codex** (`codex app-server` JSON-RPC). Reasons:
  1. No Node runtime or npx download chain. claude-agent-acp also bundles its *own* Claude native binary unless `CLAUDE_CODE_EXECUTABLE` is set.
  2. Codly gets cost, rate-limit and plan telemetry, hooks, and the full permission-mode semantics.
  3. Adapter lag disappears: claude-agent-acp is about 27k lines of TS (`src/*.ts` alone) that re-models Claude's behavior.
  4. These are the two agents Codly users will run most.
- Keep `claude-agent-acp` and `codex-acp` available as an **opt-in "ACP compat" mode**, pinned versions, for users who prefer it or to diff behavior.
- **Pi:** native `--mode rpc` (small and clean, with dollar cost). Ignore pi-acp for now.

**R2. Implement ACP with the schema crate and own the transport.** Depend on `agent-client-protocol-schema = "1.12"` (types only; enable `unstable_subagents` and `unstable_end_turn_token_usage`). Write a ~500-line tokio transport:

- `tokio::process::Command` with its own process group (`process_group(0)`) and kill-on-drop.
- `BufReader::lines()` on stdout, with stderr captured to the agent log.
- An `AtomicU64` request-ID counter, a `HashMap<Id, oneshot::Sender>` of pending requests, and a dispatcher for incoming requests and notifications.
- **Every raw line written to SQLite before parsing**, which keeps the timeline replayable and makes fixture tests trivial.
- Deserialize `session/update` permissively: keep the raw `serde_json::Value` next to the typed value, and map unknown `sessionUpdate` tags or `_meta` to `AgentEvent::Unknown{raw}` instead of erroring.

Rationale: the SDK broke its API three times in four months, and its handler model has a documented deadlock trap. Codly needs the raw tap anyway. If the team prefers the SDK, pin `agent-client-protocol = "=3.5.0"` with `features=["process","unstable_subagents"]` and isolate it behind a `trait AgentConnection`.

```rust
// Codly-side shape (sketch)
pub enum AgentEvent {            // normalized; drives BOTH views
    MessageDelta { session: SessionRef, message_id: Option<String>, text: String },
    ThoughtDelta { session: SessionRef, text: String },
    ToolStarted  { session: SessionRef, id: String, kind: ToolKind, title: String, name: Option<String>, locations: Vec<PathBuf>, raw_input: Option<Value> },
    ToolUpdated  { session: SessionRef, id: String, status: ToolStatus, diffs: Vec<Diff>, terminal: Option<TerminalChunk>, raw_output: Option<Value> },
    Plan         { session: SessionRef, entries: Vec<PlanEntry> },
    Usage        { session: SessionRef, used: u64, size: u64, cost_usd: Option<f64>, source: UsageSource },
    PermissionAsked { session: SessionRef, request: PendingRpcId, tool: ToolSnapshot, options: Vec<PermOption> },
    SubagentSpawned { parent: SessionRef, child: SessionRef, title: Option<String> },
    SubagentState   { child: SessionRef, state: RunState },
    TurnEnded    { session: SessionRef, stop: StopReason, tokens: Option<TokenUsage> },
    Unknown      { session: SessionRef, raw: Value },
}
```

**R3. Advertise these capabilities in `initialize`:**

```json
{"protocolVersion":1,
 "clientInfo":{"name":"codly","title":"Codly","version":"<app>"},
 "clientCapabilities":{
   "fs":{"readTextFile":false,"writeTextFile":false},
   "terminal":false,
   "subagents":{},
   "_meta":{"terminal_output_delta":true,"terminal_output":true}}}
```

- **fs: false.** Codly is not an editor with unsaved buffers. Delegation only adds failure modes, like the Gemini ENOENT bug seen live. Agents operate on their **git worktree** directly, and Codly observes through `notify` plus `git diff` plus `locations`. If fs delegation is ever enabled (for example, to show "agent is reading X" exactly), return `-32002` for missing files, create parent dirs on write, and confine paths to the worktree.
- **terminal: false at first, with the `_meta` terminal extension on.** That gives live command output from the Claude and Codex adapters for free. Phase 2: implement `terminal/*` on Codly's PTY manager so every ACP agent's commands show in Codly's terminal pane.
- **subagents: {}.** Each child session becomes a new character in the Living World, tied to its parent. The RFD is unstable, so gate it behind a setting and handle its absence (fall back to `Task`/`Agent` tool calls).

**R4. Handle permissions as Codly approvals.**

- Every `session/request_permission` becomes an Approval card that shows the tool snapshot, diff and locations. Answer by option **kind**. Never auto-pick `allow_always` unless the user set a standing rule.
- On `session/cancel`, answer any open permission request with `{"outcome":{"outcome":"cancelled"}}` (spec MUST).
- Time out to *reject* only if the user configured it, because unanswered requests block the agent forever. Show a "waiting for you" state on the character.
- Default each new agent session into a safe mode where the agent supports one (Gemini `default`; OpenCode `build` with a deny or ask permission config through `OPENCODE_CONFIG_CONTENT`), and rely on worktrees for blast radius.

**R5. Treat config, mode and model as a capability matrix, not assumptions.**

- On `session/new`, read `configOptions` (standard; select by `category: "model"|"mode"|"thought_level"`). Otherwise fall back to `modes` (`session/set_mode`). Otherwise fall back to the legacy `models` block (`session/set_model`, Gemini 0.62).
- Persist `(agent_id, acp_session_id, native_session_id)`. They are identical for Gemini and OpenCode [V-local]. On restart, prefer `session/resume`, then `session/load` (replays: dedupe against the stored timeline by `messageId`/`toolCallId`), then the native CLI's resume flag.

**R6. Show cost honestly.** Order of preference:

1. `usage_update.cost` (OpenCode, Claude adapter).
2. Native cost fields (OpenCode `step_finish.cost`, Pi `usage.cost.total`, Copilot `assistant.usage.cost`, Claude `total_cost_usd`).
3. Tokens × an OpenRouter price table for the API-key backend.

Otherwise show tokens only and label the agent "subscription". Claude's `total_cost_usd` under a subscription is an API-equivalent figure, not a charge. Gemini over ACP has tokens only (`_meta.quota`), and Cursor has none.

**R7. Living-World mapping, driven only by real events.**

| Event | Animation |
|---|---|
| `tool_call.kind` read or search | reading or searching at the bookshelf |
| edit, delete or move | typing at a desk |
| execute | the terminal console (with live output from `_meta.terminal_output_delta`) |
| fetch | the "internet" window |
| think, or `agent_thought_chunk` | a thought bubble showing the reasoning summary |
| `plan` | the whiteboard |
| `PermissionAsked` | a hand raised plus a speech bubble |
| `usage_update` | a "fuel" gauge |
| `subagent_update` | a new desk spawns |
| `TurnEnded` | idle |

Gemini's `update_topic` pseudo-tool supplies a free "current intent" caption.

**R8. Detection runs in two tiers** (§10).

- **Tier 1 at startup:** PATH, then `--version`, then the status command. Everything runs in parallel, with a 5 s timeout each and results cached in SQLite for 10 minutes.
- **Tier 2 on demand:** when the user first opens an agent, run the ACP handshake probe and keep that process warm as the session's agent.
- Use the ACP registry JSON to *offer* installs of missing agents, with versions pinned and sha256-verified for binaries. Never auto-run `npx -y …@latest`.

**R9. Pin the protocol and write fixture tests.**

- Support protocol v1 only. Add v2 behind a flag once it is out of draft. Model message IDs, upserts and idle state internally now so v2 is a transport change.
- Replay `fixtures/acp-*.jsonl` through the Rust transport in unit tests (a2c lines as input; c2a lines as expected output).
- Run `acp-tck` against Codly's client in CI with OpenCode's free model.

---

## 12. Fixtures captured (all in `docs/research/fixtures/`)

| File | What | Cost |
|---|---|---|
| `gemini-stream.jsonl` | `gemini -m gemini-2.5-flash -p "Reply with exactly: pong" --output-format stream-json --skip-trust` (4 lines) | paid #1 |
| `gemini-stream-tool.jsonl` | Same with "Read README.md and reply DONE" (tool_use/tool_result, `update_topic`) | paid #3 |
| `acp-gemini.jsonl` | Full ACP session against `gemini --acp -m gemini-2.5-flash --skip-trust`: initialize, session/new, prompt (read README) | paid #2 |
| `acp-gemini-write.jsonl` | ACP write and shell turn: **2 `session/request_permission`**, `fs/read_text_file` delegation with the ENOENT failure, `fs/write_text_file`, diff content | paid #4 |
| `opencode-run.jsonl` | `opencode run --format json -m opencode/big-pickle "Reply with exactly: pong"` | free model |
| `opencode-run-tool.jsonl` | Same with a read tool | free |
| `acp-opencode.jsonl` | Full ACP session against `opencode acp` (configOptions, usage_update, prompt usage, session/list) | free |
| `acp-opencode-write.jsonl` | ACP write and bash turn (no permission prompts with default config) | free |
| `acp-probe.mjs` | The Node probe that produced the `acp-*` files. Usage: `node acp-probe.mjs <out.jsonl> <cwd> <model\|-> -- <agent cmd…>`, env `PROBE_PROMPT`, `PROBE_TIMEOUT_MS`. Line format: `{"t":ms,"dir":"c2a"\|"a2c"\|"stderr"\|"note","msg"\|"text"}` | n/a |

The fixtures contain absolute scratch paths, the model's thought summaries, and the user's skill names (from `available_commands_update`). Redact them before publishing the repo.

---

## 13. Open questions and things not verified

- The exact Copilot `--output-format json` record schema (it is probably the SDK session events). Install Copilot and capture a fixture.
- Cursor `agent acp` live behavior: the `authenticate` flow, whether `usage_update` is emitted, and `cursor/*` frequency. Install and capture.
- Whether Gemini accepts a `-32002` reply and then creates the file (I expect yes from the source string match).
- Whether Gemini 0.65 (latest) added `sessionCapabilities` or `configOptions`. Re-run the probe after upgrading.
- The cause of OpenCode's first-run `run --format json` 120 s stall.
- Whether claude-agent-acp's subagent sessions and codex-acp's subagents behave identically with a non-Zed client. Test with `clientCapabilities.subagents: {}`.

## 14. Sources

- ACP spec repo and schema: https://github.com/agentclientprotocol/agent-client-protocol (commit 66ff807, schema v1.26.0; `schema/v1/meta.json`, `schema.json`, `schema.unstable.json`; `docs/announcements/acp-v2-draft.mdx`, `session-usage-stabilized.mdx`; `docs/rfds/subagents.mdx`; `docs/get-started/agents.mdx`)
- ACP registry: https://github.com/agentclientprotocol/registry (commit ee673c9; `*/agent.json`, `AUTHENTICATION.md`, `FORMAT.md`, `quarantine.json`) and https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json
- Rust SDK: https://github.com/agentclientprotocol/rust-sdk (commit 5ef0fa3), https://crates.io/crates/agent-client-protocol, https://crates.io/crates/agent-client-protocol-schema
- Claude adapter: https://github.com/agentclientprotocol/claude-agent-acp (commit 739dde3, v0.95.0)
- Codex adapter: https://github.com/agentclientprotocol/codex-acp (commit a77829f, v2.2.2)
- Gemini CLI: local `gemini --help` (0.62.0) and the installed bundle `@google/gemini-cli/bundle/*.js` (`acpFileSystemService`, `JsonStreamEventType`, `apiKeyCredentialStorage`)
- OpenCode: local `opencode --help`, `opencode serve` `/doc` OpenAPI; https://github.com/anomalyco/opencode (tag v1.19.4: `packages/opencode/src/cli/cmd/run.ts`, `packages/sdk/js/src/{index,server}.ts`)
- Cursor: https://cursor.com/docs/cli/reference/output-format, https://cursor.com/docs/cli/acp, https://cursor.com/docs/cli/reference/parameters
- Copilot CLI: https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-programmatic-reference, https://docs.github.com/en/copilot/reference/copilot-cli-reference/acp-server, https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference, https://github.com/github/copilot-cli/blob/main/changelog.md, https://github.com/github/copilot-sdk (`nodejs/src/generated/session-events.ts`)
- Pi: https://github.com/badlogic/pi-mono (commit d5b3de2; `packages/coding-agent/docs/{json,rpc,rpc-commands,providers}.md`), https://github.com/svkozak/pi-acp (v0.0.34)
- npm registry metadata (`npm view`) for package versions and deprecations, 2026-10-25
