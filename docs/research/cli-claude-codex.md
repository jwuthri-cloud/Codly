# Headless and streaming integration: Claude Code and OpenAI Codex CLI

*Research for Codly. Date: 2026-10-25. Machine: macOS 26.5 (Darwin 25.5.0), arm64.*

**Scope.** This report covers the exact headless and streaming surfaces that a Rust host (Tauri 2) needs in order to drive Claude Code and the OpenAI Codex CLI through the user's existing subscription logins. It covers flags, event shapes, approvals, cost and usage, cancel, resume, and parallel worktrees, and ends with a common event schema and opinionated recommendations.

**Evidence labels used throughout:**

- **[V-local]**: verified on this machine (CLI `--help`, a real run, or a schema generated from the installed binary).
- **[V-src]**: verified in source code (cloned repo or a downloaded npm tarball, read-only).
- **[V-doc]**: verified in official documentation fetched today.
- **[Claim]**: secondary source or vendor-reported, not independently verified.
- **[Unverified]**: my inference. Test it before relying on it.

---

## 0. TL;DR decisions

| Need | Claude Code (2.1.309) | Codex CLI (0.161.0) |
|---|---|---|
| **Transport to use** | `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio`. This is one long-lived process per agent, speaking NDJSON on stdin/stdout, and it is the same wire protocol the official Agent SDKs use. | `codex app-server` (stdio, JSONL JSON-RPC without the `"jsonrpc"` field). One long-lived process per Codly instance can host N threads, one per agent and worktree. |
| Streaming text | `stream_event` → `content_block_delta.text_delta` | `item/agentMessage/delta` |
| Tool visibility | `assistant.tool_use` blocks plus `user.tool_result` blocks, and `stream_event` `input_json_delta` | `item/started` and `item/completed` with `commandExecution`, `mcpToolCall`, and others; `item/commandExecution/outputDelta` |
| File-change visibility | Derived from Edit/Write tool inputs and `tool_use_result`. There is **no native file-change event**, so Codly's fs watcher plus git diff is the source of truth. | Native `fileChange` items (path, kind, diff) and `turn/diff/updated` (aggregated unified diff) |
| Approvals to the Codly UI | Control protocol: the CLI sends `control_request{subtype:"can_use_tool"}` and Codly answers with `control_response`. A documented fallback is `--permission-prompt-tool mcp__codly__approve`. | Server→client JSON-RPC requests `item/commandExecution/requestApproval` and `item/fileChange/requestApproval`, answered with `{decision}` |
| Cost and usage | `result.total_cost_usd` (a client-side **list-price estimate**), `usage`, `modelUsage`; `rate_limit_event` (5h and 7d utilization) | `thread/tokenUsage/updated` (tokens only, no USD); `account/rateLimits/updated` (5h and weekly `usedPercent`) |
| Cancel | `control_request{subtype:"interrupt"}` (verified free). SIGINT also ends the turn. SIGTERM exits 143 with no result. | `turn/interrupt {threadId, turnId}`, after which `turn/completed` reports `status:"interrupted"` (verified) |
| Resume | `--session-id <uuid>` on the first run, then `--resume <uuid>` (works from any cwd), `--fork-session` | `thread/resume {threadId}`, `thread/fork`; for exec, `codex exec resume <id>` |
| Fallback path | Agent SDK (TS 0.3.310 / Py 0.2.174), but the SDK bundles its own CLI binary | `codex exec --json`. It has **no text deltas** and approvals are forced to `never` [V-src]. |

**Biggest risks**, each explained in its section below:

1. **Anthropic subscription policy has moved several times in 2026.** Today's text allows an end user to sign in to the *unmodified* `claude` binary with their own subscription, but forbids third-party apps from routing Pro/Max credentials for their users. Codly must never touch tokens and needs a first-class API-key fallback (§2.12).
2. **`--bare` does not read OAuth.** Anthropic plans to make bare mode the default for `-p` in a future release. Codly must pin its behaviour and detect this (§2.8).
3. **Inherited environment variables silently change behaviour.**
   - `ANTHROPIC_API_KEY` overrides the subscription.
   - `CLAUDE_CODE_CHILD_SESSION` excludes sessions from `--resume`.
   - `CLAUDE_CODE_ENTRYPOINT` changes the reported entrypoint.

   I observed this directly: fixture (a) was polluted by the environment of the parent Claude Desktop process (§5.9).
4. **macOS GUI apps launch with a minimal PATH.** An nvm-installed `codex` is a `#!/usr/bin/env node` shim and fails without `node` on PATH (§5.9).

---

## 1. Method and environment

**Installed versions [V-local]:**
- `claude` 2.1.309: `/Users/julien/.local/bin/claude`, a symlink to `~/.local/share/claude/versions/2.1.309`.
- `codex-cli` 0.161.0: an nvm global, `~/.nvm/versions/node/v22.23.3/bin/codex`, which is a symlink to `@openai/codex/bin/codex.js`, a Node shim. The shim spawns `@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`.

**Latest versions published today [V-local via `npm view` / `gh api`]:**

| Package | Latest | Published |
|---|---|---|
| `@anthropic-ai/claude-code` | 2.1.310 | 2026-10-23 |
| `@anthropic-ai/claude-agent-sdk` | 0.3.310 | 2026-10-23 |
| `claude-agent-sdk` (PyPI) | 0.2.174 | bundles CLI 2.1.310 |
| `@openai/codex` / `@openai/codex-sdk` | 0.164.0 | 2026-10-23 |
| Codex git tags | `rust-v0.164.0`, `rust-v0.165.0-alpha.1` | 2026-10-23 |

Both CLIs ship roughly weekly, so treat every shape below as versioned.

**Auth status [V-local]:**
- `claude auth status` prints JSON: `{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","subscriptionType":"team",...}`.
- `codex login status` prints `Logged in using ChatGPT`.
- The app-server's `account/read` returns `{"account":{"type":"chatgpt","planType":"plus",...}}`.

**Sources read:**
- `openai/codex` at commit `970111f6` (2026-10-23)
- `anthropics/claude-agent-sdk-python` at commit `0e89b298` (2026-10-24)
- the `@anthropic-ai/claude-agent-sdk@0.3.310` tarball, read via `npm pack` (no install scripts run)
- the Codex protocol generated from the *installed* binary with `codex app-server generate-ts --out …` and `generate-json-schema --out …` (96 TS files plus 642 v2 types)

**Paid invocations: 4 of 4 used.** All ran in a throwaway git repo at `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/fixture-repo`, which contains `README.md` and `math_utils.py`. Each run had a 120 s timeout.

| # | Fixture | Command (abridged) | Result |
|---|---|---|---|
| a | `fixtures/claude-pong.jsonl` | `claude -p "Reply with exactly: pong" --output-format stream-json --verbose --model haiku --strict-mcp-config` | 11 lines, 5.0 s, `$0.0039` list estimate. **Caveat:** this run inherited the Claude Desktop parent environment (`startup_timing.entrypoint:"claude-desktop"`, extra Artifact tools, two `commands_changed` lines of **187 KB each**). |
| b | `fixtures/claude-tool.jsonl` | Same, with a clean `env -i` environment, prompt "Use the Read tool to read README.md then reply DONE", plus `--include-partial-messages --allowedTools Read` | 41 lines, 6.3 s, 2 turns, `$0.0037` |
| c | `fixtures/codex-exec.jsonl` | Clean environment: `codex exec --json --ignore-user-config -m gpt-6-luna -c model_reasoning_effort=low "Run the shell command ls … reply with exactly: DONE"` | 6 lines, 8.6 s |
| d | `fixtures/codex-appserver.jsonl` | Node probe ([script](#appendix-a-the-app-server-probe)) that logs every JSON-RPC line. It runs `initialize`, `account/read`, `account/rateLimits/read`, `model/list`, and `thread/start` (`approvalPolicy:"untrusted"`, `sandbox:"workspace-write"`). Turn 1 is `ls` plus an `apply_patch` file creation, with both approvals auto-accepted. Turn 2 is interrupted with `turn/interrupt`. | 95 lines, 9.4 s |
| – | `fixtures/claude-control-protocol.jsonl` | **Free** (no user message, so no inference): stream-json control requests `initialize`, `interrupt`, `set_permission_mode`, `get_context_usage` | Redacted (account and org fields) |

Stderr files: `claude-*.stderr.txt` are empty. `codex-exec.stderr.txt` contains `Reading additional input from stdin...`.

---

## 2. Claude Code

### 2.1 Flags that matter, as listed by `claude --help` in 2.1.309 [V-local]

| Flag | Notes for Codly |
|---|---|
| `-p, --print` | Non-interactive mode. Skips the workspace-trust dialog. In this mode, settings files that fail validation are **silently ignored**. |
| `--output-format text\|json\|stream-json` | Codly uses `stream-json`, and always passes `--verbose` alongside it. The docs pair them, and the SDK always sends `--output-format stream-json --verbose` ([subprocess_cli.py:574](https://github.com/anthropics/claude-agent-sdk-python/blob/main/src/claude_agent_sdk/_internal/transport/subprocess_cli.py)) [V-src]. |
| `--input-format text\|stream-json` | `stream-json` gives a long-lived multi-turn process and enables the control protocol. |
| `--include-partial-messages` | Emits `stream_event` lines that wrap raw Anthropic SSE events. |
| `--include-hook-events` | Emits hook lifecycle events (`hook_started`/`hook_progress`/`hook_response`). SessionStart and Setup hook events are always included. |
| `--replay-user-messages` | Echoes stdin user messages back on stdout. Requires stream-json in both directions. |
| `--forward-subagent-text` | Forwards subagent text and thinking blocks, tagged with `parent_tool_use_id`. |
| `--session-id <uuid>` / `-r, --resume [id\|name\|path.jsonl]` / `-c, --continue` / `--fork-session` / `--no-session-persistence` / `-n, --name` | Session control. See §2.6. |
| `--permission-mode acceptEdits\|auto\|bypassPermissions\|manual\|dontAsk\|plan` | `manual` is an alias for `default`. |
| `--permission-prompts host\|none` | Default is `host`, meaning the SDK host or `--permission-prompt-tool` answers. `none` auto-denies. Requires ≥ 2.1.259 [V-doc]. |
| `--permission-prompt-tool <mcp tool \| stdio>` | Not printed in `--help`, but documented in the [CLI reference](https://code.claude.com/docs/en/cli-reference) [V-doc]. The value `stdio` is what the SDK passes when `canUseTool` is set [V-src, `types.py:2304`]. |
| `--allowedTools` / `--disallowedTools` / `--tools` | Use rule syntax such as `"Bash(git diff *)"`. `--tools` restricts the built-in tool set. To block MCP tools, use `--disallowedTools "mcp__*"`. |
| `--mcp-config <files\|json...>` / `--strict-mcp-config` | `-p` waits up to `MCP_TIMEOUT` (default 30 s) for pending servers [V-doc]. |
| `--system-prompt[-file]`, `--append-system-prompt[-file]`, `--append-subagent-system-prompt[-file]` | Prefer `--append-system-prompt` so Claude Code's own system prompt stays in place. |
| `--add-dir <dirs...>` | Gives tool access to extra directories. |
| `--model <alias\|id>`, `--fallback-model`, `--effort low\|medium\|high\|xhigh\|max` | The `initialize` response lists models: `default`/`opus` → `claude-opus-5-5`, `sonnet` → `claude-sonnet-5-5`, `haiku` → `claude-haiku-5-5`, `claude-fable-5-1[1m]` [V-local]. |
| `--max-turns N`, `--max-budget-usd X`, `--json-schema '<schema>'` | Print mode only. The budget is checked against the client-side cost estimate. |
| `--settings <file\|json>`, `--setting-sources user,project,local` | Use these to keep untrusted repo settings out (see §5.9). |
| `--bare` | **Does not read OAuth or the keychain**, so it is unusable with subscriptions (§2.8). |
| `--restricted`, `--safe-mode` | Restricted removes code-running tools. Safe mode disables all customizations. |
| `-w, --worktree`, `--bg`, `--cloud`, `--remote-control` | Don't use these. Codly manages worktrees and processes itself. `--bg` is rejected together with `-p` [V-doc]. |

Documented but not shown in `--help` [V-doc]: `--max-turns`, `--permission-prompt-tool`, `--system-prompt-file`, `--append-system-prompt-file`. The SDK also passes internal flags such as `--thinking`, `--max-thinking-tokens`, `--resume-session-at`, `--task-budget` and `--session-mirror` [V-src]. Don't depend on them.

### 2.2 Output: `--output-format stream-json --verbose`

The output is NDJSON: one JSON object per line, discriminated by `type` and optionally `subtype`.

The full union of possible messages, from `SDKMessage` in `sdk.d.ts` of `@anthropic-ai/claude-agent-sdk@0.3.310` [V-src]:

| `type` | `subtype` values |
|---|---|
| `assistant` | (none) |
| `user` | (none; also used for the replay variant) |
| `result` | `success`, `error_during_execution`, `error_max_turns`, `error_max_budget_usd`, `error_max_structured_output_retries` |
| `stream_event` | (none) |
| `system` | `init`, `status`, `api_retry`, `compact_boundary`, `hook_started`, `hook_progress`, `hook_response`, `plugin_install`, `task_started`, `task_progress`, `task_updated`, `task_notification`, `background_tasks_changed`, `thinking_tokens`, `session_state_changed`, `worker_shutting_down`, `commands_changed`, `notification`, `files_persisted`, `memory_recall`, `elicitation_complete`, `permission_denied`, `informational`, `local_command_output`, `control_request_progress`, `model_refusal_fallback`, `model_refusal_no_fallback`, `mirror_error` |
| `tool_progress` | (none) |
| `tool_use_summary` | (none) |
| `auth_status` | (none) |
| `rate_limit_event` | (none) |
| `prompt_suggestion` | (none) |
| `conversation_reset` | (none) |

On stdout, control messages also appear: `control_request`, `control_response` and `control_cancel_request` (§2.5).

**The union is open.** Fixture (a) contains `system/post_turn_summary`, which is *not* in the 0.3.310 union. The parser must therefore pass unknown `type`/`subtype` values through rather than fail.

#### Observed order in fixture (b), with a clean environment and partial messages [V-local]

| Line(s) | Event |
|---|---|
| 0–3 | `system/hook_started` ×2, `system/hook_response` ×2 (the user's SessionStart hooks) |
| 4 | `system/init` (≈25 KB) |
| 5 | `system/status` `{"status":"requesting"}` |
| 6 | `stream_event` `message_start` (includes `usage`, `ttft_ms`, `api_message_id`) |
| 7 | `stream_event` `content_block_start` `{type:"tool_use", id:"toolu_016o…", name:"Read", input:{}}` |
| 8–25 | `stream_event` `content_block_delta` `input_json_delta` ×18 |
| 26 | **`assistant`**: the full message with `tool_use`. It arrives **before** that block's `content_block_stop`. |
| 27–29 | `stream_event` `content_block_stop`, `message_delta` (`stop_reason:"tool_use"`, usage), `message_stop` |
| 30 | `rate_limit_event` |
| 31 | `user`: the `tool_result`, plus `tool_use_result` and `tool_result_meta` |
| 32–39 | `system/status`, then text streaming (`content_block_start` text, a `text_delta` of "DONE"), then `assistant` (text), `content_block_stop`, `message_delta` (`end_turn`), `message_stop` |
| 40 | **`result/success`** |

#### Key shapes, trimmed from the fixtures

```jsonc
// system/init (fixture b; arrays truncated)
{"type":"system","subtype":"init","cwd":"/…/fixture-repo","session_id":"e3bef2cd-…",
 "tools":["Task","Bash","CronCreate",…],          // 28 tools with a clean env; 31 with the inherited one
 "mcp_servers":[],"model":"claude-haiku-5-5","permissionMode":"default",
 "apiKeySource":"none",                          // "none" means OAuth/subscription billing
 "claude_code_version":"2.1.309","output_style":"Concise",
 "slash_commands":[…],"agents":[…],"skills":[…],"plugins":[{name,path,source,version}…],
 "capabilities":["interrupt_receipt_v1","interrupt_cancel_queued_v1","interrupt_send_now_v1",
   "interrupt_requeue_taken_v1","msg_lifecycle_v1","request_marker_lists_v1",
   "lifecycle_wakeup_source_v1","sdk_mcp_tools_list_changed","sdk_mcp_manifests",
   "mcp_read_resource_v1","mcp_tool_ui_meta_v1","ui_surface_v1","mcp_call_result_not_for_model_v1"],
 "memory_paths":{"auto":"~/.claude/projects/<sanitized-cwd>/memory/"}, "uuid":"…"}

// assistant (full message; content blocks are text | thinking | redacted_thinking | tool_use | server_tool_use …)
{"type":"assistant","message":{"model":"claude-haiku-5-5","id":"msg_011CgPgYwx…","role":"assistant",
  "content":[{"type":"tool_use","id":"toolu_016o…","name":"Read",
              "input":{"file_path":"/…/fixture-repo/README.md"},"caller":{"type":"direct"}}],
  "usage":{"input_tokens":2,"cache_creation_input_tokens":15990,"cache_read_input_tokens":12037,"output_tokens":16,…}},
 "parent_tool_use_id":null,"session_id":"e3bef2cd-…","uuid":"79c1…","timestamp":"2026-10-25T21:41:38.227Z",
 "request_id":"req_011…","wire_tool_inputs":{"toolu_016o…":{"file_path":"…"}}}

// user tool_result (structured result in tool_use_result; permission provenance in tool_result_meta)
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_016o…","type":"tool_result",
   "content":"1\t# Fixture Repo\n2\t\n3\tTiny repo used to capture CLI event fixtures for Codly.\n4\t"}]},
 "parent_tool_use_id":null,
 "tool_use_result":{"type":"text","file":{"filePath":"…/README.md","content":"…","numLines":4,"startLine":1,"totalLines":4}},
 "tool_result_meta":[{"id":"toolu_016o…","permission_decision":{"decision":"accept","source":"config","reason_type":"rule"}}]}

// stream_event (partial; wraps the Anthropic Messages SSE event verbatim)
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"DONE"}},
 "session_id":"…","parent_tool_use_id":null,"uuid":"…","api_message_id":"msg_011CgPgZ5D…"}

// rate_limit_event (subscription windows)
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1792982400,"rateLimitType":"five_hour",
  "overageStatus":"rejected","overageDisabledReason":"out_of_credits","isUsingOverage":false,
  "unifiedWindows":{"five_hour":{"utilization":0,"resetsAt":1792982400},"seven_day":{"utilization":0.01,"resetsAt":1793286000}}}}

// result (fixture b, trimmed)
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"DONE","stop_reason":"end_turn",
 "duration_ms":4296,"duration_api_ms":2743,"ttft_ms":1883,"session_id":"e3bef2cd-…",
 "total_cost_usd":0.00370874,
 "usage":{"input_tokens":4,"cache_creation_input_tokens":16221,"cache_read_input_tokens":40064,"output_tokens":127,
          "output_tokens_details":{"thinking_tokens":0},"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},…},
 "modelUsage":{"claude-haiku-5-5":{"inputTokens":4,"outputTokens":127,"cacheReadInputTokens":40064,
     "cacheCreationInputTokens":16221,"costUSD":0.00370874,"contextWindow":1000000,"maxOutputTokens":128000,
     "costBasis":"list",…}},
 "permission_denials":[],"terminal_reason":"completed","subagent_stats":{…}}
```

Other documented system events [V-doc, [headless](https://code.claude.com/docs/en/headless)]:

- **`system/api_retry`**: fields `attempt`, `max_retries`, `retry_delay_ms`, `error_status`, and `error`. The `error` value is one of `authentication_failed`, `oauth_org_not_allowed`, `account_on_hold`, `billing_error`, `rate_limit`, `overloaded`, `invalid_request`, `model_not_found`, `server_error`, `max_output_tokens`, `cloud_credential_error`, or `unknown`.
- **`system/permission_denied`**: fields `tool_name`, `tool_use_id`, `decision_reason_type`, `message`.
- **`init.mcp_server_errors` and `init.plugin_errors`**: these keys are omitted when there are no errors.

**Subagents.** Messages from subagents carry `parent_tool_use_id`, which is the id of the Agent or Skill `tool_use` that started them. A forked skill started with `/<skill>` carries a value prefixed `forked-command-` instead. Following these ids rebuilds the agent tree. This feeds the "explainable orchestration" view directly.

### 2.3 Partial messages (`stream_event`)

Each `stream_event.event` is a raw Anthropic SSE event: `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`. The delta types are:

- `text_delta`: observed.
- `input_json_delta` (`partial_json`): observed. It starts with an empty string.
- `thinking_delta` and `signature_delta`: [Claim] from the Anthropic streaming API. They were not captured, because Haiku did not think in these runs.

`message_start` additionally carries `thinking_display:"updates"` and `ttft_ms` [V-local].

**Dedupe rule.** The full `assistant` message for a given `message.id` repeats content that was already streamed. Key streamed blocks by `(api_message_id, index)` and key `tool_use` blocks by `id`. Treat the `assistant` message as authoritative and the deltas as a preview. The `assistant` message can arrive before `content_block_stop`, so do not wait for the stop event before finalizing.

### 2.4 Cost semantics

`total_cost_usd` and `modelUsage[*].costUSD` are **client-side estimates at list price** (`costBasis:"list"` observed). The headless docs warn that these figures may not match the actual bill. For subscription users the number is an API-equivalent value, not spend.

On `--continue`/`--resume`, the `result` reports the conversation's cumulative total including earlier runs [V-doc]. So display **per-turn deltas** computed by Codly (current minus previous), not the raw value.

### 2.5 Streaming input and the control protocol (`--input-format stream-json`)

The protocol is NDJSON on stdin. The message shapes come from the SDK source [V-src, `_internal/client.py:187`, `client.py:250`]:

```jsonc
// user turn (write one line; the process stays alive; each turn ends with a `result` line)
{"type":"user","session_id":"","message":{"role":"user","content":"Fix the failing test"},"parent_tool_use_id":null}
// content may also be an array of Anthropic content blocks (text/image). SDKUserMessage also has
// optional `priority: "now"|"next"|"later"` and `uuid` for queued "steer" messages (capability interrupt_send_now_v1).

// host → CLI control request
{"type":"control_request","request_id":"req_1","request":{"subtype":"initialize","hooks":null}}
// CLI → host control response
{"type":"control_response","response":{"subtype":"success","request_id":"req_1","response":{…}}}
// error form
{"type":"control_response","response":{"subtype":"error","request_id":"req_1","error":"…"}}
```

Control requests I verified for free on 2.1.309 [V-local, `fixtures/claude-control-protocol.jsonl`]:

| Request | Response |
|---|---|
| `initialize` | Keys `account{apiProvider,email,organization,subscriptionType}`, `models[{value,resolvedModel,displayName,supportedEffortLevels,…}]`, `commands[349]`, `agents`, `output_style`, `available_output_styles`, `current_permission_mode`, `claude_code_version`, `pid`, `capabilities`, `session_state:"idle"`. This lets Codly **detect the login, plan, and model list without spending tokens.** |
| `interrupt` | `{"still_queued":[]}` (the `interrupt_receipt_v1` capability) |
| `set_permission_mode {mode:"acceptEdits"}` | `{"mode":"acceptEdits"}`, followed by `system/status` with `permissionMode` |
| `get_context_usage` | `{categories:[{id:"system_prompt",tokens:3075,…},…]}`. This feeds a context meter. |

Other subtypes defined in the 0.3.310 types [V-src]: `set_model`, `set_max_thinking_tokens`, `mcp_status`, `mcp_set_servers`, `mcp_reconnect`, `mcp_toggle`, `mcp_call`, `rewind_files`, `stop_task`, `get_usage`, `get_session_cost`, `get_settings`, `apply_flag_settings`, `reload_plugins`, `rename_session`, `cancel_async_message`, `list_models`, and more.

**`system/init` timing.** With stream-json input, `system/init` is emitted only once the first user message is processed. Control-only sessions showed no `init` line [V-local].

**CLI → host requests** (you must answer them) [V-src, `_internal/query.py:586-700`, `types.py:2801`]:

```jsonc
{"type":"control_request","request_id":"<cli id>","request":{
  "subtype":"can_use_tool","tool_name":"Bash","input":{"command":"npm test","description":"…"},
  "tool_use_id":"toolu_…","permission_suggestions":[/* PermissionUpdate[] */],
  "blocked_path":null,"decision_reason":"…","title":"…","display_name":"…","description":"…","agent_id":"…"}}
// TS types add: decision_reason_type, classifier_approvable, suppress_always_allow_rule, default_to_no,
// matched_ask_rule, mcp_server, requires_user_interaction

// answer: allow (updatedInput optional since v2.1.207; updatedPermissions = echo a suggestion to persist a rule)
{"type":"control_response","response":{"subtype":"success","request_id":"<cli id>",
  "response":{"behavior":"allow","updatedInput":{…},"updatedPermissions":[…]}}}
// answer: deny (interrupt:true also stops the turn)
{"type":"control_response","response":{"subtype":"success","request_id":"<cli id>",
  "response":{"behavior":"deny","message":"User declined: use pnpm","interrupt":false}}}
```

The other CLI → host request subtypes are:
- `hook_callback`: only appears if you registered SDK hooks in `initialize`.
- `mcp_message`: only appears for in-process "SDK MCP servers".
- `control_cancel_request`: the CLI abandoned a pending request, so close the approval card.

`AskUserQuestion` arrives through the same `can_use_tool` path with `tool_name:"AskUserQuestion"`. The `input.questions[]` items have `question`, `header`, 2–4 `options{label,description}`, and `multiSelect`. Answer with `allow` and `updatedInput:{questions, answers:{<question text>: <label or labels>}}` [V-doc, [approvals](https://code.claude.com/docs/en/agent-sdk/approvals)].

**Stability.** Anthropic does not document the control protocol as a standalone public API. It is fully typed in the public `sdk.d.ts`, both official SDKs depend on it, and the CLI advertises `capabilities` for feature detection. I judge it stable enough to depend on, but gate behaviour on `capabilities` and on `claude_code_version` [Unverified risk assessment].

### 2.6 Sessions

- **Pre-assign IDs.** Pass `--session-id <uuid v4>` on the first run, so the Codly DB row exists before the process starts. Afterwards use `--resume <uuid>`. Resume works from **any** working directory, because the CLI finds the session by ID across projects. It also accepts an absolute path to a transcript `.jsonl` [V-doc].
- **Fork.** `--resume <id> --fork-session` creates a new session ID, which gives "branch this agent" for free.
- **Transcript location.** `~/.claude/projects/<sanitized cwd>/<session-id>.jsonl`, and auto memory lives under the same project directory [V-local, `init.memory_paths`]. Each git worktree has a different cwd, so it gets a different project directory. Override with `CLAUDE_CONFIG_DIR` plus `CLAUDE_CODE_PROJECT_DIR_NAME` [V-doc].
- **Interrupted turns.** After an interrupt, resume leaves the interrupted turn as it was. Setting `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1` makes the model continue that turn [V-doc].
- **Nested sessions.** A session started with `CLAUDE_CODE_CHILD_SESSION=1` inherited is excluded from `--resume`, `--continue` and history [V-doc, env-vars]. Scrub this variable (§5.9).
- **Persistence off.** `--no-session-persistence`, or `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1`, means no transcript is written and the session cannot be resumed.

### 2.7 Permissions

The documentation is [permission-modes](https://code.claude.com/docs/en/permission-modes) [V-doc].

- **Starting mode.** On ≥ 2.1.302, `-p` and SDK runs with no mode set start in **Manual** (`default`), and anything that needs approval is denied unless an allow rule, hook, `canUseTool`, or `--permission-prompt-tool` approves it. Both fixtures show `permissionMode:"default"` [V-local]. The SDK `.d.ts` comment still mentions `auto` as a possible default, so **always pass `--permission-mode` explicitly.**
- **Mode summary:**

  | Mode | Runs without asking |
  |---|---|
  | `default` | Reads only |
  | `acceptEdits` | Plus edits and `mkdir`/`touch`/`mv`/`cp` |
  | `plan` | Reads only, no edits until a plan is approved |
  | `auto` | Most actions; a classifier reviews them |
  | `dontAsk` | Pre-approved actions only; everything else is denied |
  | `bypassPermissions` | Everything (containers only) |

- **Never auto-approved in any mode:** explicit ask rules, `AskUserQuestion`, MCP tools marked `requiresUserInteraction`, critical-path `rm`/`rmdir`, and some reads outside the working directories.
- **Evaluation order.** The approvals doc notes that the callback is skipped for tools that are already auto-approved. A `PreToolUse` hook runs before everything else and can allow, deny, modify, or `defer`.
- **MCP permission tool** (`--permission-prompt-tool mcp__codly__approve`). This is a documented public contract. The tool receives the tool name, input, and tool-use id, and returns a text content item containing JSON `{"behavior":"allow","updatedInput":{…}}` or `{"behavior":"deny","message":"…"}`. Claude Code waits for that MCP server to connect before turn 1. An `allow` for an MCP tool that requires user interaction is converted to deny (≥ 2.1.199) [V-doc].
- **Denials in the stream.** Denials appear as `system/permission_denied` messages and in `result.permission_denials[]`.

### 2.8 Other knobs

- **`--bare` / `CLAUDE_CODE_SIMPLE=1`.** In this mode the CLI does **not** read OAuth or the keychain, so it only works with `ANTHROPIC_API_KEY` or an `apiKeyHelper`. The docs say bare mode is planned to become the default for `-p` [V-doc, headless].
  - **Codly impact:** in subscription mode, never pass `--bare`, and make sure `CLAUDE_CODE_SIMPLE` is unset or `0`. The inherited environment here had `CLAUDE_CODE_SIMPLE=0`.
  - **Detecting the switch:** if the default flips, `-p` runs will fail auth. Check `apiKeySource` and the `system/api_retry` `authentication_failed` error, and watch the changelog for a flag that opts out.
- **Untrusted repo content.** Without `--bare`, `-p` loads `~/.claude` and project `.claude/settings.json` hooks, `.mcp.json` servers, CLAUDE.md, plugins and skills, *with no trust prompt*, even in folders the user has never trusted [V-doc].
- **Project setting controls.** `--setting-sources user` skips project settings, and `--strict-mcp-config` skips `.mcp.json`. For the OpenRouter/API-key backend, `--bare` plus explicit flags is ideal.
- **Background work at exit.** After the final turn, `-p` may stay open up to 10 minutes waiting on background tasks (`CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`). With stream-json input Codly controls the lifetime anyway: close stdin to end the process.
- **Slow consumers.** If the consumer reads slowly, the CLI waits up to 30 s for queued output to drain before exiting. Always drain stdout promptly.
- **Stdin cap.** Piped stdin is capped at 10 MB [V-doc].

### 2.9 Cancellation

- **Preferred:** send `control_request{subtype:"interrupt"}`, optionally with `cancel_queued:true` (`interrupt_cancel_queued_v1`). The turn ends, a `result` line follows, and the process stays alive for the next turn. The free call is verified; mid-turn behaviour was not exercised.
- **SIGINT** ends the turn, with the same effect as the SDK's `interrupt()` [V-doc].
- **SIGTERM** exits with code 143 and **records no result for the in-progress turn**. It kills the process tree of a running Bash command and runs `SessionEnd` hooks. A pending permission prompt is left unanswered [V-doc].
- **Codly sequence:** interrupt via control request, wait up to 5 s for `result`, close stdin, wait, send SIGTERM to the process group, and finally SIGKILL.

### 2.10 Claude Agent SDK as an alternative

The packages are `@anthropic-ai/claude-agent-sdk@0.3.310` (TypeScript) and `claude-agent-sdk==0.2.174` (Python, bundles CLI 2.1.310).

Neither SDK talks to the API directly. Both spawn the CLI with the flags above plus `--input-format stream-json` [V-src]. Details:

- **TypeScript binary.** The TS SDK installs a **bundled native CLI** through optional dependencies (`@anthropic-ai/claude-agent-sdk-darwin-arm64`, …). Set `pathToClaudeCodeExecutable` to use the user's own `claude` instead [V-src, `sdk.d.ts:2017`].
- **Environment.** The SDK sets `CLAUDE_CODE_ENTRYPOINT=sdk-py` / `sdk-ts` and `CLAUDE_AGENT_SDK_VERSION` [V-src].
- **Line buffer.** The Python SDK's default `max_buffer_size` is **1 MB** per JSON line [V-src, `subprocess_cli.py:36`]. Fixture (a) had a 187 KB line, and tool results can exceed 1 MB.
- **Feature parity.** The SDK adds callbacks (`canUseTool`, hooks, in-process MCP tools), but a Rust host gets everything the SDK gets by speaking the same NDJSON directly.

**Verdict:** don't embed Node or Python just to wrap a protocol Rust can speak directly. Keep the TS SDK only as a reference implementation and as a test oracle.

### 2.11 Auth detection

- `claude auth status` prints JSON with `loggedIn`, `authMethod` (for example `"claude.ai"`), `apiProvider`, `subscriptionType`, and `configDirectory` [V-local]. Use it for onboarding checks.
- `system/init.apiKeySource` is `"none"` for OAuth/subscription. Any other value means API-key billing, so warn the user.
- `ANTHROPIC_API_KEY` in the environment **takes precedence over the subscription even when the user is logged in** [V-doc, env-vars]. `CLAUDE_CODE_OAUTH_TOKEN` exists for automation, and `claude setup-token` mints one. Codly must not mint, read, or store tokens (§2.12).

### 2.12 Anthropic policy on subscriptions in third-party apps (as of 2026-10-25)

This is not legal advice.

**Primary source: [Legal and compliance](https://code.claude.com/docs/en/legal-and-compliance), fetched today [V-doc].** Paraphrasing:

- OAuth is for subscription purchasers, for ordinary use of Claude Code and other native Anthropic apps.
- Developers building products, including Agent SDK products, should use API keys.
- Third-party developers may not offer Claude.ai login in their apps, may not route requests through Free/Pro/Max credentials on behalf of their users, and may not collect, store, or intermediate Claude credentials. Sign-in must complete through Anthropic's own flow.
- The page carves out an end user "signing in to the unmodified Claude Code binary with their own Claude subscription", including when a platform hosts Claude Code.
- Products that run Claude Code must not modify the binary or disable its auth methods, and must not resell or intermediate usage.
- Advertised Pro/Max limits assume ordinary, individual usage of Claude Code and the Agent SDK. Anthropic may enforce these rules without notice.

**[Support: "Use the Claude Agent SDK with your Claude plan"](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan) [V-doc]:**
- 2026-06-15 update: previously announced Agent SDK usage changes were paused.
- 2026-10-07 update: Max and Team plans include monthly API credits that cover the Agent SDK and `claude -p`, and `claude -p` and third-party apps can still use subscription limits.

**[Consumer Terms](https://www.anthropic.com/legal/consumer-terms), effective 2025-10-08 [V-doc]:** automated or non-human access is prohibited, except via an API key or where Anthropic explicitly permits it.

**Timeline [Claim, secondary sources]:**
- February 2026: a terms clarification on third-party access, per [The Register](https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access/).
- 2026-04-04: subscription limits stopped covering third-party harnesses such as OpenClaw, with "extra usage" required instead. Sources: Boris Cherny on [X](https://x.com/bcherny/status/2040206441756471399) and [claude-mem #1826](https://github.com/thedotmack/claude-mem/issues/1826).
- These events predate the June pause and the October credits above.

**What this means for Codly:**
1. The defensible shape is that the user installs `claude` and runs `claude auth login` themselves, and Codly spawns the **unmodified** binary. Codly never reads the keychain, never handles `CLAUDE_CODE_OAUTH_TOKEN` or `setup-token`, and never proxies or resells.
2. Heavy parallel orchestration may exceed what Anthropic considers ordinary individual usage. Cap concurrency, show the subscription-window usage, and offer a one-click switch to API-key mode (`ANTHROPIC_API_KEY`, or `--bare` with an API key) and to the OpenRouter backend.
3. Re-check this page and the support article at each release. Expose a kill switch that remotely disables subscription mode if the policy changes.

---

## 3. OpenAI Codex CLI

### 3.1 Flags as listed by `codex --help` in 0.161.0 [V-local]

**Global flags:**

| Flag | Notes |
|---|---|
| `-c key=value` | The value is parsed as TOML. |
| `--enable` / `--disable <feature>` | Feature flags. |
| `-m, --model` | Model selection. |
| `-p, --profile` | Config profile. |
| `-s, --sandbox read-only\|workspace-write\|danger-full-access` | Sandbox mode. |
| `-a, --ask-for-approval on-request\|never` | Approval policy. |
| `--approve-for-me` | Routes approvals to the auto-review subagent. |
| `--dangerously-bypass-approvals-and-sandbox` | No approvals and no sandbox. |
| `--dangerously-bypass-hook-trust` | Runs hooks without persisted trust. |
| `-C, --cd`, `--add-dir` | Working root; extra writable directories. |
| `--worktree` | Don't use; Codly manages worktrees. |
| `--search` | Live web search. |
| `--remote ws://\|unix://`, `--no-daemon` | The TUI uses a **shared local app-server daemon** by default. |

**Subcommands:** `exec`, `review`, `login`, `logout`, `mcp`, `plugin`, `app-server`, `remote-control`, `app`, `resume`, `fork`, `queue`, `archive`, `delete`, `cloud`, `exec-server`, `features`, `sandbox`, `debug`, `doctor`, `agents`.

**`codex exec [PROMPT]`** (alias `e`) has the same model and sandbox flags, plus:
- `--json` (the alias `--experimental-json` is what the TS SDK uses)
- `-o, --output-last-message <file>`
- `--output-schema <file>`
- `--skip-git-repo-check`
- `--ephemeral`
- `--ignore-user-config` (auth still comes from `CODEX_HOME`)
- `--ignore-rules`
- `--thread-source`
- `--color`
- `-i, --image`

There is **no `-a` flag on exec**: headless exec forces `approval_policy = Never` [V-src, `exec/src/lib.rs:591`]. `PROMPT` set to `-` reads stdin. If stdin is piped as well as a prompt, stdin is appended as a `<stdin>` block, and the CLI prints `Reading additional input from stdin...` even with `</dev/null` [V-local].

**Other exec subcommands:**
- `codex exec resume [SESSION_ID|--last] [PROMPT]` takes the same flags and accepts a UUID or a thread name.
- `codex exec fork <id>`
- `codex exec review`

**`codex app-server`:**
- `--listen stdio:// (default)|unix://[PATH]|ws://IP:PORT|off` selects the transport.
- `--ws-auth capability-token|signed-bearer-token` (with `--ws-token-file`, `--ws-token-sha256`, `--ws-shared-secret-file`) authenticates WebSocket clients.
- `--analytics-default-enabled`
- Subcommands: `daemon {start,stop,restart,version,…}`, `proxy --sock`, `generate-ts --out DIR [--experimental]`, `generate-json-schema --out DIR [--experimental]`.

**Utilities:**
- `codex debug models` dumps the raw model catalog as JSON. Today it lists `gpt-6.1-sol` (the default, a general-purpose model), `gpt-6-astra` (frontier), `gpt-6-sol`, `gpt-6-luna` (the low-cost fast model), `gpt-5.6-sol/terra/luna`, plus the hidden `gpt-reserve` and `codex-auto-review` [V-local].
- Effort levels are `low|medium|high|xhigh|max`, plus `ultra` on some models.

### 3.2 `codex exec --json` JSONL schema

Source: `codex-rs/exec/src/exec_events.rs` [V-src]. Top level is `#[serde(tag="type")]`:

| `type` | Payload |
|---|---|
| `thread.started` | `{thread_id}` (UUIDv7; use it to resume) |
| `turn.started` | `{}` |
| `turn.completed` | `{usage:{input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens}}` |
| `turn.failed` | `{error:{message}}` |
| `item.started` / `item.updated` / `item.completed` | `{item:{id, type, …}}` |
| `error` | `{message}` (unrecoverable) |

Item `type` values are `snake_case`, flattened next to `id`:

| Item `type` | Fields |
|---|---|
| `agent_message` | `text` (a JSON string when `--output-schema` is used) |
| `reasoning` | `text` (summary) |
| `command_execution` | `command`, `aggregated_output`, `exit_code?`, `status: in_progress\|completed\|failed\|declined` |
| `file_change` | `changes:[{path, kind: add\|delete\|update}]`, `status: in_progress\|completed\|failed`. **No diff text**, and it is emitted only on completion. |
| `mcp_tool_call` | `server`, `tool`, `arguments`, `result?{content[], structured_content, _meta?}`, `error?{message}`, `status` |
| `collab_tool_call` | `tool: spawn_agent\|send_input\|wait\|close_agent`, `sender_thread_id`, `receiver_thread_ids`, `prompt?`, `agents_states{}`, `status` |
| `web_search` | `id`, `query`, `action`, `results?` |
| `todo_list` | `items:[{text, completed}]`. This is the only item that gets `item.updated`. |
| `error` | `message` (non-fatal) |

Real output from fixture (c) [V-local]:

```json
{"type":"thread.started","thread_id":"01a17504-c425-7380-860d-f70769120ba9"}
{"type":"turn.started"}
{"type":"item.started","item":{"id":"item_0","type":"command_execution","command":"/bin/zsh -lc ls","aggregated_output":"","exit_code":null,"status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_0","type":"command_execution","command":"/bin/zsh -lc ls","aggregated_output":"README.md\nmath_utils.py\n","exit_code":0,"status":"completed"}}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"DONE"}}
{"type":"turn.completed","usage":{"input_tokens":30046,"cached_input_tokens":23040,"cache_write_input_tokens":0,"output_tokens":31,"reasoning_output_tokens":0}}
```

**Limitations, from source** [V-src, `event_processor_with_jsonl_output.rs`]:
- No `agentMessage` deltas, so you only get the final message text.
- No `turn/diff`.
- No approvals (policy forced to `never`).
- No rate-limit info and no USD cost.

`codex exec` is itself a thin wrapper around an **in-process app-server client**. It uses `InProcessAppServerClient` and turns Ctrl-C into `turn/interrupt` [V-src, `lib.rs:1192,1313`]. This is why the app-server is the real integration surface.

The other SDKs follow the same split: `@openai/codex-sdk@0.164.0` (TypeScript) spawns `codex exec --experimental-json` plus `--config` overrides [V-src, `sdk/typescript/src/exec.ts`], while the Python SDK (`sdk/python`) talks to the app-server.

### 3.3 `codex app-server` JSON-RPC protocol

**Transport [V-doc, [learn.chatgpt.com/docs/app-server](https://learn.chatgpt.com/docs/app-server); V-local]:**
- stdio, one JSON message per line.
- JSON-RPC 2.0 semantics, but **the `"jsonrpc":"2.0"` field is omitted on the wire.**
- Requests have `{method, id, params}`, notifications have `{method, params}`, and responses have `{id, result|error}`.
- Server notifications also carry a top-level `emittedAtMs` field [V-local].
- The WebSocket transport is experimental. The docs warn that non-loopback WS listeners are unauthenticated by default during rollout.

**Handshake [V-local]:**

```json
→ {"method":"initialize","id":0,"params":{"clientInfo":{"name":"codly","title":"Codly","version":"0.1.0"},"capabilities":{"experimentalApi":false}}}
← {"id":0,"result":{"userAgent":"codly_probe/0.161.0 (Mac OS 26.5.2; arm64) dumb (codly_probe; 0.0.1)","codexHome":"/Users/julien/.codex","platformFamily":"unix","platformOs":"macos"}}
→ {"method":"initialized"}
← {"method":"remoteControl/status/changed","params":{"status":"disabled",…},"emittedAtMs":…}
← {"method":"account/updated","params":{"authMode":"chatgpt","planType":"plus"},"emittedAtMs":…}
```

Requests sent before the handshake get a `Not initialized` error, and a second `initialize` gets `Already initialized` [V-doc]. The `capabilities` object also accepts:
- `optOutNotificationMethods: string[]`, to suppress noisy notifications (`remoteControl/status/changed`, `mcpServer/startupStatus/updated`, …)
- `requestAttestation`
- `extensions`

Methods and fields marked experimental require `experimentalApi:true`.

**Client→server methods in 0.161.0** come from the generated `ClientRequest.ts`, 105 in total [V-local]. The core set for Codly is:

| Method | Params, then result |
|---|---|
| `thread/start` | `{model?, modelProvider?, serviceTier?, cwd?, approvalPolicy?: "untrusted"\|"on-request"\|"never"\|{granular:{…}}, approvalsReviewer?: "user"\|"auto_review"\|"guardian_subagent", sandbox?: "read-only"\|"workspace-write"\|"danger-full-access", config?: {<toml key>: json}, baseInstructions?, developerInstructions?, ephemeral?, threadSource?}` → `{thread, model, modelProvider, cwd, approvalPolicy, approvalsReviewer, sandbox, reasoningEffort, instructionSources, …}` |
| `turn/start` | `{threadId, input: UserInput[], cwd?, approvalPolicy?, sandboxPolicy?, model?, effort?, summary?, outputSchema?, clientUserMessageId?, …}` → `{turn:{id, status:"inProgress", items:[], …}}`. Per-turn overrides persist to later turns. |
| `turn/steer` | Appends input to the active turn (needs `expectedTurnId`) [V-doc]. |
| `turn/interrupt` | `{threadId, turnId}` → `{}` |
| `thread/resume` | `{threadId, …overrides, excludeTurns?}`. A running thread is rejoined. |
| `thread/fork`, `thread/read`, `thread/list`, `thread/loaded/list`, `thread/turns/list`, `thread/items/list`, `thread/archive`, `thread/delete`, `thread/name/set`, `thread/revert`, `thread/compact/start`, `thread/unsubscribe` | Thread lifecycle and history. |
| `review/start` | Runs a code review. |
| `account/read {refreshToken?}` | → `{account:{type:"chatgpt",email,planType}, requiresOpenaiAuth, workspaceRouting}` |
| `account/rateLimits/read` | → `{ordinaryUsageAllowed, rateLimits:{limitId:"codex", primary:{usedPercent, windowDurationMins:300, resetsAt}, secondary:{usedPercent, windowDurationMins:10080, resetsAt}, credits, planType, rateLimitReachedType}, rateLimitsByLimitId, rateLimitResetCredits}` |
| `account/login/start {type:"chatgpt"\|"chatgptDeviceCode"\|"apiKey", …}`, `account/logout` | Login is via a browser URL returned as `authUrl`, followed by the `account/login/completed` notification. |
| `model/list {limit?, includeHidden?}` | → `{data:[{id, displayName, supportedReasoningEfforts[], defaultReasoningEffort, isDefault, …}]}`. The docs say this is a catalog, **not an entitlement check**. |
| `config/read`, `config/value/write`, `mcpServerStatus/list`, `skills/list`, `command/exec`, `fs/*`, `gitDiffToRemote`, `fuzzyFileSearch` | Utilities. |

The `UserInput` union:

```ts
type UserInput =
  | { type: "text", text: string, text_elements: TextElement[] }
  | { type: "image", url } | { type: "image", fileId }
  | { type: "localImage", path }
  | { type: "audio", url } | { type: "localAudio", path }
  | { type: "skill", name, path }
  | { type: "mention", name, path };
```

**Server→client requests** need a JSON-RPC response from Codly:

| Method | Params | Response |
|---|---|---|
| `item/commandExecution/requestApproval` | `{kind, threadId, turnId, itemId, startedAtMs, approvalId?, environmentId, reason?, networkApprovalContext?, command?, cwd?, commandActions?, proposedExecpolicyAmendment?, proposedNetworkPolicyAmendments?}`. The observed message also carried **`availableDecisions`**. | `{decision: "accept"\|"acceptForSession"\|{acceptWithExecpolicyAmendment:{execpolicy_amendment:[…]}}\|{applyNetworkPolicyAmendment:{…}}\|"decline"\|"cancel"}` |
| `item/fileChange/requestApproval` | `{threadId, turnId, itemId, startedAtMs, reason?, grantRoot?}` | `{decision: "accept"\|"acceptForSession"\|"decline"\|"cancel"}` |
| `item/permissions/requestApproval` | `{threadId, turnId, itemId, cwd, reason, permissions}` | `{permissions, scope, strictAutoReview?}` |
| `item/tool/requestUserInput` | A question to the user. | Answer object |
| `mcpServer/elicitation/request` | MCP form or URL elicitation. | Elicitation result |
| `item/tool/call` | Dynamic tools Codly registers. | Tool result |
| `account/chatgptAuthTokens/refresh`, `attestation/generate` | Only appear when opted in. | — |
| `applyPatchApproval`, `execCommandApproval` | Legacy v1 forms. | — |

**Server notifications** come from the generated `ServerNotification.ts`, 95 in total [V-local]. The core set:

| Group | Methods |
|---|---|
| Thread | `thread/started`, `thread/status/changed` (`{type:"active", activeFlags:["waitingOnApproval"]}` or `{type:"idle"}`), `thread/closed`, `thread/tokenUsage/updated`, `thread/compacted` |
| Turn | `turn/started`, `turn/completed` (with `turn.status: completed\|interrupted\|failed\|inProgress` and `turn.error{message, codexErrorInfo, additionalDetails}`), `turn/diff/updated` (`{diff}`, the turn's aggregated unified diff), `turn/plan/updated` (`{explanation, plan:[{step, status: pending\|inProgress\|completed}]}`) |
| Item lifecycle | `item/started`, `item/completed`. The docs call the completed item the authoritative state. |
| Item deltas | `item/agentMessage/delta`, `item/plan/delta`, `item/reasoning/summaryTextDelta` (`{delta, summaryIndex}`), `item/reasoning/summaryPartAdded`, `item/reasoning/textDelta`, `item/commandExecution/outputDelta`, `item/commandExecution/terminalInteraction`, `item/fileChange/patchUpdated`, `item/mcpToolCall/progress`. `item/fileChange/outputDelta` is deprecated. |
| Approvals | `serverRequest/resolved` (`{threadId, requestId}`), `item/autoApprovalReview/started` and `/completed` |
| Account | `account/rateLimits/updated`, `account/updated`, `account/login/completed` |
| Errors and warnings | `error` (`{error:TurnError, willRetry, threadId, turnId}`), `warning`, `configWarning`, `deprecationNotice`, `model/rerouted` |
| Other | `mcpServer/startupStatus/updated`, `hook/started`, `hook/completed` |

**`ThreadItem` variants (v2, camelCase `type`):**

| Variant | Fields |
|---|---|
| `userMessage` | — |
| `agentMessage` | `{text, phase: "commentary"\|"final_answer"\|null, …}`. `commentary` is progress narration, which is ideal for the pixel-office speech bubbles. |
| `reasoning` | `{summary[], content[]}` |
| `plan` | — |
| `commandExecution` | `{command, cwd, processId, source, status, commandActions:[{type:"read"\|"listFiles"\|"search"\|"unknown", …}], aggregatedOutput, exitCode, durationMs}` |
| `fileChange` | `{changes:[{path, kind:{type:"add"\|"delete"\|"update", move_path?}, diff}], status}` |
| `mcpToolCall` | `{server, tool, arguments, result, error, durationMs, …}` |
| `dynamicToolCall` | — |
| `collabAgentToolCall` | `{tool, senderThreadId, receiverThreadIds, prompt, model, agentsStates}` |
| `subAgentActivity` | — |
| Others | `webSearch`, `imageView`, `imageGeneration`, `sleep`, `functionCallOutput`, `hookPrompt`, `enteredReviewMode`, `exitedReviewMode`, `contextCompaction` |

**`CodexErrorInfo` values:** `contextWindowExceeded`, `sessionBudgetExceeded`, `usageLimitExceeded`, `rateLimitExceeded`, `serverOverloaded`, `unauthorized`, `sandboxError`, `httpConnectionFailed{httpStatusCode}`, `responseStreamDisconnected{…}`, `activeTurnNotSteerable{…}`, `tooManyDenials`, `other`, and others.

#### Observed sequence from fixture (d), turn 1 (`approvalPolicy:"untrusted"`, `workspace-write`) [V-local]

| t (ms) | Message |
|---|---|
| 741 | `thread/start` result (thread `01a17505-fa34-…`, `path: ~/.codex/sessions/2026/10/25/rollout-…jsonl`, `source:"vscode"`, `historyMode:"paginated"`), then `turn/start` → `{turn:{id:"01a17505-fa93-…", status:"inProgress"}}` |
| 741–1998 | `thread/started`, then `mcpServer/startupStatus/updated` ×7. **The user's `config.toml` MCP servers started even with `-c 'mcp_servers={}'`.** |
| 1128 | `thread/status/changed` (active), `turn/started` |
| 2208 | `item/started` and `item/completed` for `userMessage` |
| 3213–3483 | `item/started` agentMessage (`phase:"commentary"`), `item/agentMessage/delta` ×18, `item/completed` ("I'll run `ls`, then create `notes.txt` with `apply_patch`.") |
| 4163 | `thread/status/changed` `activeFlags:["waitingOnApproval"]`, `item/started` commandExecution (`/bin/zsh -lc ls`, `commandActions:[{type:"listFiles"}]`), then `item/commandExecution/requestApproval` id 0 with `proposedExecpolicyAmendment:["ls"]` and `availableDecisions:["accept",{acceptWithExecpolicyAmendment…},"cancel"]` |
| 4164 | Codly sends `{"id":0,"result":{"decision":"accept"}}`, then receives `serverRequest/resolved` and `thread/status/changed` (active, no flags) |
| 4267 | `item/completed` commandExecution (`aggregatedOutput:"README.md\nmath_utils.py\n"`, `exitCode:0`) |
| 4270 | `thread/tokenUsage/updated` (`total` and `last` breakdowns, `modelContextWindow:258400`), `account/rateLimits/updated` |
| 6350–6351 | `item/started` fileChange (`notes.txt`, kind `add`, diff `"hello\n"`), `waitingOnApproval`, then `item/fileChange/requestApproval` id 1. Codly sends `accept`, then receives `serverRequest/resolved` |
| 6394–6395 | `item/completed` fileChange, then `turn/diff/updated` with a real `diff --git a/notes.txt b/notes.txt …` |
| 7812–7957 | agentMessage (`phase:"final_answer"`) "DONE" |
| 8048 | `thread/status/changed` idle, `turn/completed` `{status:"completed", durationMs:7298}` |

Turn 2 (interrupt test): `turn/start` → `turn/started` → `item/started` agentMessage → the first `delta` → Codly sends `turn/interrupt` → `{}` (5 ms later) → `thread/status/changed` idle → `turn/completed {status:"interrupted", items:[], durationMs:1277}`.

**Schema drift control.**
- Codly's CI should run `codex app-server generate-json-schema --out <dir>` against the minimum supported and the latest Codex versions, and diff the results.
- The crates.io crate `codex-app-server-protocol` (0.63.0, 2025-12-11) is published from a **fork** (`namastexlabs/codex`), so it is stale and unofficial. **Do not depend on it.** Use hand-written serde types for the subset in §5.3, falling back to `serde_json::Value`.

### 3.4 Sandbox, approvals, config

**Sandbox.**
- `read-only` is the default for exec.
- `workspace-write` makes cwd and `writableRoots` writable. Network is off by default; turn it on with `sandbox_workspace_write.network_access = true`.
- `danger-full-access` removes the sandbox.

The app-server's `turn/start.sandboxPolicy` accepts:

```jsonc
{"type":"workspaceWrite","writableRoots":[…],"networkAccess":false,"excludeTmpdirEnvVar":false,"excludeSlashTmp":false}
```

**Approval policy.**
- The config schema in `core/config.schema.json` [V-src] allows `on-request`, `{granular:{sandbox_approval, rules, skill_approval, request_permissions, mcp_elicitations}}`, and `never`.
- The app-server protocol type also accepts `"untrusted"`, which **worked in 0.161.0**: even `ls` required approval. Because `untrusted` is missing from the config schema, I treat it as legacy and possibly deprecated [Unverified].
- `approvalsReviewer: "auto_review"` (also `--approve-for-me`) routes approvals to a reviewer model instead of the user.

**Useful `-c` keys** [V-src, config schema]:
- `model`, `model_reasoning_effort` (any value the model advertises)
- `model_reasoning_summary` (`auto|concise|detailed|none`), `model_verbosity`
- `approval_policy`, `sandbox_mode`, `sandbox_workspace_write.{writable_roots,network_access}`
- `shell_environment_policy.*`
- `mcp_servers.<name>.enabled` (also `enabled_tools`, `disabled_tools`, `startup_timeout_sec`, `tool_timeout_sec`)
- `web_search` (`disabled|cached|indexed|live`)
- `developer_instructions`, `hide_agent_reasoning`, `show_raw_agent_reasoning`
- `history.*`, `features.*`, `cli_auth_credentials_store` (`file|keyring|auto|ephemeral`)

The app-server takes the same keys via `thread/start.config`.

**Project trust and hooks.** `thread/start` does not persist trust for directories that have no project marker [V-doc]. Hooks require persisted trust unless `--dangerously-bypass-hook-trust` is passed.

### 3.5 Sessions

- Thread IDs are UUIDv7. Each thread is stored as a rollout file at `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<threadId>.jsonl`, and the `thread.path` field gives the path [V-local].
- `thread/resume {threadId}` works after an app-server restart. `thread/fork` branches a thread.
- For exec: `codex exec resume <id> "<prompt>"`.
- `ephemeral: true` (or `--ephemeral`) persists nothing.
- One app-server process hosts many threads (`thread/loaded/list`), and each thread has its own `cwd`, sandbox and approval policy.

### 3.6 Codex auth and third-party terms

**Auth [V-doc, [learn.chatgpt.com/docs/auth](https://learn.chatgpt.com/docs/auth)]:**
- Sign in with ChatGPT (subscription) or with an API key (usage billing).
- Credentials live in `~/.codex/auth.json`, a plaintext file, or in the OS keyring, depending on `cli_auth_credentials_store`. `CODEX_HOME` defaults to `~/.codex`.
- Headless login: `codex login --device-auth`.
- `CODEX_API_KEY=… codex exec` uses an API key for a single run. Exec reuses saved CLI auth by default.

**OpenAI's sanctioned path for third-party apps [V-doc]:**
- "Sign in with ChatGPT" (SIWC) with plan usage is documented at [developers.openai.com/siwc/token-sharing-open-source](https://developers.openai.com/siwc/token-sharing-open-source) and [its codex-app-server page](https://developers.openai.com/siwc/token-sharing-open-source/codex-app-server).
- Open-source and locally hosted apps can request permission to use the user's ChatGPT plan.
- The app registers through the browser using a persisted `ext_agent_host_id`, then passes the OAuth `access_token` to `codex app-server` as `ACCESS_TOKEN`, using a custom provider (`openai_chatgpt_plan`, `requires_openai_auth=false`). The access token is sent as a Bearer token to `/v1/responses`.
- `clientInfo.name` must stay stable across installs, because it becomes the request originator.
- Paid or remotely hosted apps must complete an interest form.
- Users can cap each app's share of their weekly plan usage ([learn.chatgpt.com/docs/sign-in-with-chatgpt](https://learn.chatgpt.com/docs/sign-in-with-chatgpt)).
- Secondary sources report a DevDay 2026 launch with about 16 partners, including OpenCode [Claim].

**What I did not find [Unverified]:** any OpenAI text that explicitly allows or forbids a third-party app spawning the user's own, already logged-in `codex` CLI or app-server. The app-server is clearly designed for integrators (the IDE extension, the Python SDK), and the local `--help` text describes `--analytics-default-enabled` as intended for OpenAI's own clients such as the VS Code extension.

**Recommendation:** v1 spawns the user's own `codex` login, which is equivalent to the user running Codex in another frontend. If Codly ships paid or closed-source, apply for SIWC so the integration is explicitly sanctioned.

**Environment gotcha:** this machine had `OPENAI_API_KEY` exported in the parent environment. Scrub `OPENAI_API_KEY` and `CODEX_API_KEY` in subscription mode, so a run never silently bills the API (§5.9).

---

## 4. ACP adapters (for comparison)

| Package | Version | Notes |
|---|---|---|
| `@agentclientprotocol/claude-agent-acp` | 0.95.0 (2026-10-24) | Wraps `@anthropic-ai/claude-agent-sdk@0.3.309`, which brings its **own bundled CLI binary**. It replaces `@zed-industries/claude-code-acp`, which is deprecated. |
| `@agentclientprotocol/codex-acp` | 2.2.2 (2026-10-23) | Depends on `@openai/codex ^0.160.1` and `vscode-jsonrpc`. It replaces `@zed-industries/codex-acp`, which is deprecated. |
| `agent-client-protocol` (Rust crate) | 3.5.0 (2026-10-22) | Official Rust SDK. |

All rows are [V-local] via `npm view` or the crates.io API.

ACP gives one protocol for every agent, which is attractive for Gemini, OpenCode and others. For Claude and Codex specifically it is a lossy, Node-hosted extra layer:

- **Claude through ACP loses** `rate_limit_event`, `total_cost_usd`/`modelUsage`, `capabilities`, the control requests (`get_context_usage`, `set_model`), and subagent `parent_tool_use_id` fidelity.
- **Codex through ACP loses** `turn/diff/updated`, `commandActions`, `phase`, and account rate limits.
- **Both:** each adapter adds a Node runtime and an extra version to pin.

**Verdict:** use native adapters for Claude and Codex, and ACP for long-tail agents. Shape Codly's internal enum (§5.3) so an ACP adapter maps onto it too.

---

## 5. Recommendations for Codly

### 5.1 Decisions

1. **Claude: drive `claude` directly.** Run one long-lived process per agent:
   ```
   claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages
     --permission-prompt-tool stdio --permission-mode default
     --session-id <uuid>          (first run)   |   --resume <uuid>   (later)
     --model <alias> [--effort <lvl>] [--max-turns N] [--max-budget-usd X]
     --strict-mcp-config --mcp-config <codly-mcp.json>
     --append-system-prompt <codly role/task brief>
     [--add-dir <shared dirs>] [--setting-sources user,local]   (untrusted repos: drop "project")
     -n "codly:<agent-name>"
   ```
   Don't use the Agent SDK, `--bare`, `--worktree` or `--bg`. Speak the NDJSON control protocol from Rust.
2. **Codex: one `codex app-server` (stdio) child per Codly instance**, spawned with `-c` overrides as needed. Create one **thread per agent** with `cwd` set to the agent's worktree.
   - Spawn a private app-server; don't attach to the user's shared daemon, so Codly controls the version and lifecycle. A later "show my Codex app sessions" feature could use `codex app-server proxy`.
   - Use `codex exec --json` only as a degraded fallback, for example on a version where the app-server breaks.
3. **Normalize both into one Rust `AgentEvent` stream** (§5.3). Persist the **raw line** next to each normalized event in SQLite, so the timeline can replay and the "Living Agent World" can be re-derived after mapping fixes.
4. **The fs watcher plus `git diff` in each worktree is the source of truth for file changes.** Provider file events are hints. Claude has no native file-change event; Codex has `fileChange` and `turn/diff`. Use provider events for attribution (which agent, which tool call) and Codly's watcher for the content.
5. **Approvals all go through one Rust `ApprovalBroker`** (§5.5), never through each CLI's own UI.
6. **Treat the subscription as the scarce resource.**
   - Track both providers' rate windows. Claude reports `five_hour` and `seven_day` utilization; Codex reports `primary` (300 min) and `secondary` (10080 min) `usedPercent`.
   - Default to at most 3 concurrent agents per provider, which the user can change.
   - Back off on `api_retry` with `rate_limit`, and on Codex `usageLimitExceeded`/`rateLimitExceeded`.

### 5.2 Process supervisor (Rust)

```rust
// Cargo deps verified on crates.io today:
// tokio = { version = "1.54", features = ["process","io-util","macros","rt-multi-thread","sync","time","signal"] }
// tokio-util = { version = "0.7.20", features = ["codec"] }   futures = "0.3"
// serde = { version = "1.0.229", features = ["derive"] }  serde_json = { version = "1.0.154", features = ["raw_value"] }
// uuid = { version = "1.29", features = ["v4","v7","serde"] }  thiserror = "2.0.21"  tracing = "0.1.44"
// nix = { version = "0.31.3", features = ["signal","process"] }   which = "8.0.6"   dirs = "7.0.0"
// (optional) process-wrap = "10.0.1" for process-group/kill-tree helpers

use tokio::process::{Command, Child};
use tokio_util::codec::{FramedRead, LinesCodec};
use futures::StreamExt;

const MAX_LINE: usize = 64 * 1024 * 1024; // init is ~25KB; commands_changed hit 187KB; tool results can be MBs

fn spawn_jsonl(mut cmd: Command) -> std::io::Result<Child> {
    cmd.stdin(std::process::Stdio::piped())
       .stdout(std::process::Stdio::piped())
       .stderr(std::process::Stdio::piped())
       .kill_on_drop(true);
    #[cfg(unix)] { cmd.process_group(0); }   // own pgid -> killpg() reaps tool subprocesses too
    cmd.spawn()
}

async fn pump_stdout(child: &mut Child, tx: tokio::sync::mpsc::UnboundedSender<String>) {
    let out = child.stdout.take().unwrap();
    let mut lines = FramedRead::new(out, LinesCodec::new_with_max_length(MAX_LINE));
    while let Some(item) = lines.next().await {
        match item {
            Ok(line) if !line.trim().is_empty() => { let _ = tx.send(line); }  // never block the reader
            Ok(_) => {}
            Err(e) => tracing::warn!(?e, "oversized/invalid line dropped"),   // codec resyncs at next '\n'
        }
    }
}
// Also drain stderr in its own task (Codex app-server writes ANSI tracing logs there; a full 64KB pipe
// would stall the child). Set NO_COLOR=1 for Codex.
```

**Shutdown ladder.** Apply these steps in order:
1. Claude: send the interrupt control request. Codex: send `turn/interrupt`.
2. Wait 5 s for `result` or `turn/completed`.
3. Close stdin.
4. Wait 3 s.
5. `killpg(SIGTERM)`.
6. Wait 3 s.
7. `killpg(SIGKILL)`.

On the first SIGINT or SIGTERM the Codex app-server drains running turns, and a second signal forces exit [V-src, `app-server/src/lib.rs:226-311`].

### 5.3 Common event schema

```rust
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Provider { ClaudeCode, Codex, Gemini, OpenCode, Cursor, Copilot, Pi, DeepAgents }

/// Envelope persisted to SQLite (one row per event) and fanned out to both views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    pub run_id: uuid::Uuid,              // Codly agent instance
    pub seq: u64,                        // monotonic per run (ordering key for replay)
    pub at_ms: i64,                      // Codly receive time (Codex also gives emittedAtMs)
    pub provider: Provider,
    pub session_id: Option<String>,      // claude session_id | codex threadId
    pub turn_id: Option<String>,         // codex turnId | claude: Codly-assigned per user message
    pub parent_call_id: Option<String>,  // claude parent_tool_use_id | codex parentThreadId / collab call
    pub kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Box<RawValue>>,      // original provider line (debug + re-normalization)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    SessionReady { model: String, cwd: PathBuf, cli_version: String, tools: Vec<String>,
                   permission_mode: Option<String>, billing: Billing, capabilities: Vec<String> },
    TurnStarted { user_text: Option<String> },
    StatusChanged { state: AgentState },                         // drives pixel-office animation
    MessageDelta { item_id: String, text: String },
    MessageCompleted { item_id: String, text: String, phase: MessagePhase },
    ReasoningDelta { item_id: String, text: String },
    ReasoningCompleted { item_id: String, summary: String },
    ToolStarted { call_id: String, tool: ToolKind, input: Value },
    ToolInputDelta { call_id: String, partial_json: String },    // claude input_json_delta
    ToolOutputDelta { call_id: String, chunk: String },          // codex commandExecution/outputDelta
    ToolCompleted { call_id: String, status: ToolStatus, output: Option<String>,
                    exit_code: Option<i32>, duration_ms: Option<u64> },
    FilesChanged { call_id: Option<String>, changes: Vec<FileChange>, source: ChangeSource },
    TurnDiff { unified_diff: String },
    PlanUpdated { explanation: Option<String>, steps: Vec<PlanStep> },
    ApprovalRequested(ApprovalRequest),
    ApprovalResolved { approval_id: String, decision: ApprovalDecision, by: Resolver },
    Usage(UsageSnapshot),
    RateLimit(RateLimitSnapshot),
    Retrying { attempt: u32, max: Option<u32>, delay_ms: Option<u64>, reason: String },
    SubAgent { call_id: String, child_session: Option<String>, description: String, status: ToolStatus },
    TurnCompleted { status: TurnStatus, result_text: Option<String>, duration_ms: Option<u64>,
                    num_turns: Option<u32>, error: Option<String> },
    Warning { message: String },
    Error { message: String, code: Option<String>, fatal: bool },
    ProcessExited { code: Option<i32>, signal: Option<i32> },
    Unknown { provider_type: String },                           // never drop: raw is kept
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Billing { Subscription, ApiKey, ThirdPartyProvider, Unknown }
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum AgentState { Idle, Requesting, Thinking, Typing, RunningTool, WaitingOnApproval, Retrying, Compacting, Done, Failed }
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum MessagePhase { Commentary, Final, Unknown }
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ToolStatus { Running, Completed, Failed, Declined, Interrupted }
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum TurnStatus { Completed, Interrupted, Failed, MaxTurns, BudgetExceeded }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolKind {
    Shell { command: String, cwd: Option<PathBuf> },
    Read { path: PathBuf }, Edit { path: PathBuf }, Write { path: PathBuf },
    Search { query: Option<String>, path: Option<PathBuf> },
    WebSearch { query: String }, WebFetch { url: String },
    Mcp { server: String, tool: String },
    SubAgent { agent_type: Option<String> }, Todo, Question,
    Other { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange { pub path: PathBuf, pub kind: ChangeKind, pub diff: Option<String> }
#[derive(Debug, Clone, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ChangeKind { Add, Delete, Update { move_to: Option<PathBuf> } }
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ChangeSource { ProviderEvent, ToolInput, FsWatcher }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep { pub text: String, pub status: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub approval_id: String,            // claude control request_id | codex JSON-RPC id (stringified)
    pub call_id: Option<String>,        // claude tool_use_id | codex itemId
    pub subject: ApprovalSubject,
    pub reason: Option<String>,
    pub choices: Vec<ApprovalChoice>,   // codex availableDecisions / claude permission_suggestions
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalSubject {
    Command { command: String, cwd: Option<PathBuf> },
    FileChange { paths: Vec<PathBuf> },
    Tool { name: String, input: Value },
    Question { questions: Value },       // AskUserQuestion / item/tool/requestUserInput / MCP elicitation
    Permissions { detail: Value },       // codex item/permissions/requestApproval
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum ApprovalChoice { Once, ForSession, Always, Deny, Cancel }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApprovalDecision {
    Allow { scope: ApprovalChoice, updated_input: Option<Value> },
    Deny { message: Option<String> },
    Answer { answers: Value },
    Cancel,                              // also interrupts the turn
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)] #[serde(rename_all = "snake_case")]
pub enum Resolver { User, CodlyPolicy, Provider, Timeout }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub scope_total: bool,               // true = cumulative for session, false = this turn/request
    pub input_tokens: u64, pub cached_input_tokens: u64, pub cache_write_tokens: u64,
    pub output_tokens: u64, pub reasoning_tokens: u64,
    pub context_window: Option<u64>,
    pub cost_usd: Option<f64>,           // Claude only; list-price estimate
    pub cost_is_estimate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitSnapshot {
    pub window: String,                  // "five_hour" | "seven_day" | "codex.primary" | "codex.secondary"
    pub used_fraction: Option<f64>,      // claude utilization (0..1) | codex usedPercent/100
    pub resets_at: Option<i64>,          // unix seconds
    pub status: Option<String>,          // claude allowed|allowed_warning|rejected
}
```

#### 5.3.1 Mapping: Claude stream-json to `EventKind`

| Claude line | `EventKind` |
|---|---|
| `system/init` | `SessionReady`. Billing is `Subscription` when `apiKeySource == "none"`, else `ApiKey`. |
| `system/status {status:"requesting"}` | `StatusChanged(Requesting)` |
| `system/status {permissionMode}` | `Warning` or an info update |
| `stream_event` `content_block_start{text}` and `text_delta` | `StatusChanged(Typing)`, then `MessageDelta{item_id: api_message_id+":"+index}` |
| `stream_event` `thinking_delta` | `StatusChanged(Thinking)`, then `ReasoningDelta` |
| `stream_event` `content_block_start{tool_use}` and `input_json_delta` | `ToolStarted` (input `{}`), then `ToolInputDelta` |
| `assistant` text, thinking, and tool_use blocks | `MessageCompleted` (phase `Unknown`, set to `Final` when it is the last before `result`); `ReasoningCompleted`; `ToolStarted` upsert with final `input` (dedupe by `tool_use.id`). For Edit/Write/MultiEdit/NotebookEdit tool calls, also emit `FilesChanged{source:ToolInput}`. |
| `user` with `tool_result` | `ToolCompleted` (`is_error` → `Failed`). Use `tool_use_result` for structured details. |
| `control_request can_use_tool` | `ApprovalRequested` plus `StatusChanged(WaitingOnApproval)`. `AskUserQuestion` maps to `Question`. |
| `control_cancel_request` | `ApprovalResolved{by:Provider, decision:Cancel}` |
| `system/permission_denied` | `ApprovalResolved{Deny, by:Provider}` |
| `rate_limit_event` | `RateLimit` (one per `unifiedWindows` entry) |
| `system/api_retry` | `Retrying` (fatal categories `authentication_failed`, `billing_error`, `account_on_hold` map to `Error{fatal}`) |
| `system/task_started`, `task_progress`, `task_notification`, and messages with `parent_tool_use_id` | `SubAgent` and nested events |
| `system/compact_boundary` | `StatusChanged(Compacting)` |
| `result` | `Usage{scope_total:true, cost_usd, cost_is_estimate:true}`, then `TurnCompleted`. Subtype mapping: `success` → `Completed`, `error_max_turns` → `MaxTurns`, `error_max_budget_usd` → `BudgetExceeded`, `error_during_execution` → `Failed`, or `Interrupted` if Codly sent an interrupt. |
| `system/hook_*`, `commands_changed`, `post_turn_summary`, other | `Unknown` (raw kept) |

#### 5.3.2 Mapping: Codex app-server to `EventKind`

| Codex message | `EventKind` |
|---|---|
| `thread/start` result or `thread/started` | `SessionReady` (billing from `account/read`: `chatgpt` → `Subscription`, `apiKey` → `ApiKey`) |
| `turn/started` | `TurnStarted` |
| `thread/status/changed` | `StatusChanged` (`waitingOnApproval` → `WaitingOnApproval`, `idle` → `Idle`) |
| `item/started agentMessage` and `item/agentMessage/delta` | `StatusChanged(Typing)`, then `MessageDelta` |
| `item/completed agentMessage` | `MessageCompleted{phase: commentary → Commentary, final_answer → Final}` |
| `item/reasoning/summaryTextDelta` / `textDelta` and `item/completed reasoning` | `ReasoningDelta`, then `ReasoningCompleted` |
| `item/started commandExecution` | `ToolStarted{Shell}`. Use `commandActions` to refine to `Read`/`Search`. |
| `item/commandExecution/outputDelta` | `ToolOutputDelta` |
| `item/completed commandExecution` | `ToolCompleted` (`status`, `exitCode`, `durationMs`, `aggregatedOutput`) |
| `item/started fileChange` / `item/completed fileChange` | `ToolStarted{Edit}`; then `FilesChanged{ProviderEvent}` and `ToolCompleted` |
| `item/*/mcpToolCall` | `ToolStarted` / `ToolCompleted{Mcp}` |
| `collabAgentToolCall`, `subAgentActivity` | `SubAgent` |
| `turn/diff/updated` | `TurnDiff` |
| `turn/plan/updated` | `PlanUpdated` |
| Server requests `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, `item/tool/requestUserInput`, `mcpServer/elicitation/request` | `ApprovalRequested` |
| `serverRequest/resolved` | `ApprovalResolved` (if Codly didn't already record it) |
| `thread/tokenUsage/updated` | `Usage{scope_total:true}` from `total`, plus `Usage{scope_total:false}` from `last`; `context_window` from `modelContextWindow` |
| `account/rateLimits/updated` | `RateLimit` (primary and secondary windows) |
| `error {willRetry:true}` | `Retrying` |
| `error {willRetry:false}` | `Error` |
| `turn/completed` | `TurnCompleted` (`turn.status`, `turn.error`) |
| `mcpServer/startupStatus/updated`, `remoteControl/*`, other | `Unknown`, or suppress via `optOutNotificationMethods` |

The `codex exec --json` fallback maps onto the same subset. `item.completed agent_message` maps to `MessageCompleted`, and `turn.completed.usage` maps to `Usage{scope_total:false}`. There are no deltas and no approvals in this path.

### 5.4 Codex JSON-RPC client sketch

```rust
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Incoming {
    // order matters: requests have both id and method
    ServerRequest { id: serde_json::Value, method: String, #[serde(default)] params: serde_json::Value },
    Notification  { method: String, #[serde(default)] params: serde_json::Value },
    Response      { id: serde_json::Value, #[serde(default)] result: Option<serde_json::Value>,
                    #[serde(default)] error: Option<serde_json::Value> },
}
// write: serde_json::to_string(&json!({"method": m, "id": n, "params": p}))? + "\n"   (no "jsonrpc" key)
// reply: {"id": <same id as request>, "result": {"decision": "accept"}}
```

Pending client requests live in a `HashMap<i64, oneshot::Sender<Result<Value>>>`, and pending server requests in the `ApprovalBroker`. The `thread/start` params Codly should send:

```json
{"model":"gpt-6.1-sol","cwd":"/…/worktrees/agent-7","approvalPolicy":"on-request",
 "sandbox":"workspace-write","config":{"model_reasoning_effort":"medium","model_reasoning_summary":"auto"},
 "developerInstructions":"<Codly agent brief>","threadSource":"codly"}
```

The default `thread.source` was observed as `"vscode"` for app-server clients [V-local]. Set `threadSource` and check how it shows up in Codex's history UI [Unverified].

### 5.5 Approval routing (the `ApprovalBroker`)

**Flow:**

1. An adapter receives a provider request, either Claude `can_use_tool` or a Codex `requestApproval`.
2. It emits `ApprovalRequested` and stores a responder closure in the broker.
3. The broker first checks **Codly policy rules** (per project, agent and tool) and auto-resolves when one matches. Otherwise it shows a card in the Professional Workspace, and in the Living World the agent "raises a hand".
4. The user decides, and the broker maps the decision back to the provider:

| Codly decision | Claude response | Codex response |
|---|---|---|
| Once | `{"behavior":"allow"}`, with `updatedInput` when edited | `"accept"` |
| For session | Not native: Codly remembers the choice in its policy for the session, then sends `allow`. | `"acceptForSession"` |
| Always | Echo a `permission_suggestions` entry in `updatedPermissions`, or add a Codly rule. | `acceptWithExecpolicyAmendment` with `proposedExecpolicyAmendment` |
| Deny with feedback | `{"behavior":"deny","message":"…"}` | `"decline"`. Then send feedback as a `turn/steer` or the next turn. |
| Stop | `{"behavior":"deny","interrupt":true}` | `"cancel"` |

5. When the provider cancels (Claude `control_cancel_request`, or a Codex turn ending), the broker closes the card.
6. **Timeouts:** none by default, because both CLIs wait indefinitely. Show the waiting time in the UI. For unattended runs, set a per-agent policy: Claude `--permission-prompts none` or `dontAsk`, Codex `approvalPolicy:"never"` with the sandbox.

**Fallback for Claude:** `--permission-prompt-tool mcp__codly__approve`, served by Codly's own MCP server (`rmcp` 3.7.0 over streamable HTTP on 127.0.0.1 with a per-run bearer token, referenced from `--mcp-config`). Use it if a future CLI drops `stdio` permission routing. This is a documented public contract, but it is heavier: an MCP server, a startup wait of up to `MCP_TIMEOUT`, and no `control_cancel_request`.

### 5.6 Cost and usage display

- **Claude, per turn:** take the delta between consecutive `result.total_cost_usd` values (cumulative per session), labelled "≈ API list value" when billing is `Subscription`. Show tokens from `modelUsage` per model, and the context meter from `get_context_usage` (a free control request).
- **Codex, per turn:** `last` tokens. Per thread: `total`. There is no USD figure, so optionally compute an "API-equivalent" estimate from a versioned Codly price table and label it clearly.
- **Budget.** Show the subscription windows (Claude 5h/7d utilization, Codex 5h/weekly `usedPercent`) as the primary budget in the UI. The scheduler pauses new agents above a threshold, such as 85%.

### 5.7 Cancel, resume, fork

| Action | Claude | Codex |
|---|---|---|
| Cancel turn | `control_request {subtype:"interrupt", cancel_queued:true}` | `turn/interrupt {threadId, turnId}` |
| Steer mid-turn | Write a `user` message (queued; `priority` field); capability `interrupt_send_now_v1` | `turn/steer` |
| Next turn | Write a `user` line to the same process | `turn/start` |
| App restart | `--resume <session_id>` (new process) | Restart the app-server, `initialize`, `thread/resume {threadId}` |
| Branch | `--resume <id> --fork-session` | `thread/fork` |
| Kill agent | Shutdown ladder (§5.2) | `turn/interrupt`, then `thread/unsubscribe`. Keep the app-server running for other threads. |

### 5.8 Parallel instances in git worktrees

- **Claude:** run one process per worktree, with `current_dir(worktree)`. Each worktree path maps to its own `~/.claude/projects/<dir>`, and sessions resume by ID from anywhere.
- **Codex:** one app-server, one thread per worktree, with `cwd` set to the worktree. The `workspace-write` writable root is that cwd.
- **Commits in worktrees:** a worktree's `.git` is a *file* pointing at `<main>/.git/worktrees/<name>`, so `git add`/`commit` inside a sandboxed agent writes **outside** the cwd. In Codex `workspace-write`, add `<main>/.git` to `writableRoots`, or better, **let Codly perform the commits itself**. In Claude, Bash `git commit` is likely to need approval, or `--add-dir <main>/.git` [Unverified, test this].
- **Pre-flight per worktree:** check that the dependency install has run, because agents otherwise waste turns on it. Also check that ports don't collide when agents run dev servers.

### 5.9 Gotchas checklist

1. **PATH in GUI apps.** A Finder or Dock launch gets `PATH=/usr/bin:/bin:/usr/sbin:/sbin`.
   - At startup, resolve binaries by running `"$SHELL" -lic 'command -v claude; command -v codex; command -v node; printf %s "$PATH"'` (5 s timeout), and cache the captured PATH for children.
   - Let the user override the paths.
   - The `tauri-apps/fix-path-env-rs` crate is git-only (not on crates.io) and was last pushed 2025-09, so the 10 lines are better written yourself.
   - The nvm `codex` is a `#!/usr/bin/env node` shim, so either put `node` on PATH or exec the vendored native binary directly (`…/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex`).
   - `~/.local/bin/claude` is a symlink the auto-updater re-points. Resolve it at spawn time, not once at startup.
2. **Environment scrub.** Build each child's env from a clean base: `HOME`, `USER`, `LOGNAME`, `SHELL`, `TMPDIR`, `LANG`, `TERM=dumb`, the captured `PATH`, plus explicit additions. **Remove:**
   - `CLAUDECODE`, `CLAUDE_CODE_*` (especially `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_SIMPLE`, `CLAUDE_CODE_OAUTH_TOKEN`), and `CLAUDE_AGENT_SDK_*`.
   - `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_BASE_URL` in subscription mode.
   - `OPENAI_API_KEY` and `CODEX_API_KEY` in subscription mode.

   Evidence: fixture (a), run with the inherited Claude Desktop environment, showed `entrypoint:"claude-desktop"`, three extra tools, two 187 KB `commands_changed` lines, and a 4 KB hook output. Fixture (b), with a clean environment, had none of these. This machine also exported `OPENAI_API_KEY` and 30-plus `CLAUDE_CODE_*` variables [V-local]. Consider setting `DISABLE_AUTOUPDATER=1` during runs, and `NO_COLOR=1` for Codex.
3. **`CLAUDE_CONFIG_DIR` and `CODEX_HOME`.** Respect the user's values, because they may run multiple accounts this way. Never point them at a Codly-owned directory in subscription mode, since that would require a separate login [V-doc].
4. **Untrusted repositories.** Both CLIs execute project-level config (Claude: hooks, `.mcp.json`, CLAUDE.md; Codex: `.codex/`, rules, hooks once trusted). Claude does this without a trust prompt in `-p`. Codly needs its own "trust this repo" gate. For untrusted repos, use `--setting-sources user` plus `--strict-mcp-config`, and Codex `--ignore-rules` or `-c` overrides.
5. **User-level MCP servers in Codex.** These start for every thread (`heysam`, `node_repl`, `cua_repl`, `codex_apps` here), and `-c 'mcp_servers={}'` did **not** disable them. Failed servers spam stderr. Use `-c mcp_servers.<name>.enabled=false` per server, or accept the behaviour and suppress `mcpServer/startupStatus/updated` [V-local; the per-server toggle is untested].
6. **Very long lines.** Claude `init` is about 25 KB, `commands_changed` lines were 187 KB, and Read results contain whole files. Use `LinesCodec::new_with_max_length(64 MiB)` and keep `raw` as `Box<RawValue>`. Never use a 1 MB cap like the Python SDK default.
7. **Stdout buffering and backpressure.** Rust `stdout` (Codex) is line-buffered, so it is fine. Claude waits up to 30 s for its output queue to drain at exit. Read continuously into an unbounded or large channel, and never apply backpressure from UI rendering. Always drain stderr.
8. **Ordering.** Claude can emit the full `assistant` message before `content_block_stop`. Codex `item/completed` is authoritative over deltas. Order by Codly `seq`, and upsert by item or call id.
9. **Unknown events.** Both protocols add types weekly: `post_turn_summary` already falls outside the published TS union, and Codex has 95 notification types. Map unknowns to `Unknown` with `raw`, and never crash or drop.
10. **Version gates.**
    - Claude: require ≥ 2.1.302 (Manual default in `-p`) and ≥ 2.1.259 (`--permission-prompts`). Feature-detect with `init.capabilities`.
    - Codex: require ≥ 0.161 for the approval and diff shapes captured here, and diff generated schemas in CI.
    - Show "CLI too old or too new, untested" banners.
11. **Concurrency.** Many `claude -p` processes share `~/.claude.json` and the `~/.claude/` stores. Several app-server processes sharing `CODEX_HOME` and its SQLite stores is what the IDE extension and TUI already do, but prefer a single Codly-owned app-server [Unverified for write contention]. Rate limits are shared across all agents *and* the user's interactive use.
12. **Rate-limit failure modes.**
    - Claude: `system/api_retry{error:"rate_limit"}`, then possibly `result` with `is_error`. `rate_limit_event.status:"rejected"` ends work until `resetsAt`.
    - Codex: `error{codexErrorInfo:"usageLimitExceeded"|"rateLimitExceeded"}`, and `rateLimitReachedType`.
    - The scheduler should park queued agents until the reset, and offer the API-key or OpenRouter backend instead.
13. **Background-work waits in `-p`.** Claude can keep a process alive up to 10 minutes after the final turn for background tasks. With stream-json input Codly owns the lifetime. Optionally set `CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1` so the agent tree stays explainable.
14. **Telemetry and privacy.** Claude Code: `DISABLE_TELEMETRY=1` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` are user choices, so offer them as toggles. Codex app-server analytics are off unless `--analytics-default-enabled` is passed [V-local help].

### 5.10 Testing plan

- **Golden fixtures.** Replay every file in `docs/research/fixtures/` through each adapter, then snapshot-test the normalized `AgentEvent` stream with `insta`.
- **Live smoke test** (opt-in, costs money). Run the 4 invocations from §1 against the user's installed CLIs on each upgrade. Verify that `SessionReady`, at least one `ToolStarted`/`ToolCompleted`, an approval round-trip (Codex untrusted; Claude with `--permission-mode default` and a Write), an interrupt, and `TurnCompleted` all appear.
- **Drift check.** Diff `codex app-server generate-json-schema` output and Claude `init.capabilities`, `claude_code_version` and `--help` text against the last known-good versions, and fail CI with a readable diff.

---

## 6. Verified versus claimed ledger

| Statement | Status |
|---|---|
| Every Claude flag in §2.1 table (except where marked doc-only) | V-local (`--help`) |
| Claude stream-json shapes, line order, dedupe hazard, `rate_limit_event`, `result` fields, `costBasis:"list"` | V-local (fixtures a, b) |
| Control protocol `initialize`/`interrupt`/`set_permission_mode`/`get_context_usage` | V-local (free run) |
| `can_use_tool` request and response shapes; SDK passes `--permission-prompt-tool stdio` | V-src (Python SDK, TS `.d.ts`) |
| `can_use_tool` actually firing on 2.1.309 | **Not exercised.** It would need a paid run that triggers a non-auto-approved tool. Test early. |
| Manual is the default in `-p` (≥ 2.1.302); `--bare` skips OAuth and may become the default | V-doc, plus V-local (`permissionMode:"default"`) |
| Anthropic legal and support text on subscription use | V-doc (fetched 2026-10-25) |
| April 2026 third-party-harness cutoff | Claim (X posts and press) |
| Codex exec JSONL schema | V-src (`exec_events.rs`), plus V-local fixture c |
| exec forces `approval_policy=Never`; exec uses an in-process app-server | V-src |
| App-server method lists and types for 0.161.0 | V-local (generated from the installed binary) |
| Approval round-trip, `availableDecisions`, `turn/diff/updated`, interrupt → `interrupted`, `phase` commentary/final | V-local (fixture d) |
| SIWC program details | V-doc |
| OpenAI stance on spawning the user's own logged-in CLI | Not found (Unverified) |
| Worktree `.git` writes blocked by sandboxes | Unverified inference |
| crates.io versions in §5.2 | V-local (crates.io API, 2026-10-25) |
| crates.io `codex-app-server-protocol` 0.63.0 comes from a fork; `claude-agent-sdk` 0.1.1 points at a 404 repo (`anthropics/claude-agent-sdk-rust`) | V-local. Avoid both. |

---

## 7. Sources

**Anthropic**
- Headless and `-p` mode: https://code.claude.com/docs/en/headless
- CLI reference: https://code.claude.com/docs/en/cli-reference
- Permission modes: https://code.claude.com/docs/en/permission-modes
- Approvals and `canUseTool`: https://code.claude.com/docs/en/agent-sdk/approvals
- Environment variables: https://code.claude.com/docs/en/env-vars
- Legal and compliance: https://code.claude.com/docs/en/legal-and-compliance
- Agent SDK with Claude plans: https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- Consumer terms: https://www.anthropic.com/legal/consumer-terms
- Python Agent SDK source: https://github.com/anthropics/claude-agent-sdk-python (`src/claude_agent_sdk/_internal/transport/subprocess_cli.py`, `_internal/query.py`, `types.py`)
- TypeScript Agent SDK types: `@anthropic-ai/claude-agent-sdk@0.3.310` `sdk.d.ts` (npm tarball)

**OpenAI**
- Codex source: https://github.com/openai/codex (`codex-rs/exec/src/exec_events.rs`, `exec/src/lib.rs`, `exec/src/event_processor_with_jsonl_output.rs`, `app-server/src/lib.rs`, `core/config.schema.json`, `sdk/typescript/src/exec.ts`)
- Codex releases: https://github.com/openai/codex/releases (0.162 to 0.164 notes)
- App-server docs: https://learn.chatgpt.com/docs/app-server (redirected from developers.openai.com/codex/app-server)
- Non-interactive mode: https://learn.chatgpt.com/docs/non-interactive-mode
- Auth: https://learn.chatgpt.com/docs/auth
- Sign in with ChatGPT: https://learn.chatgpt.com/docs/sign-in-with-chatgpt
- SIWC for open-source apps: https://developers.openai.com/siwc/token-sharing-open-source and its `/codex-app-server` and `/preview-limitations` pages

**Secondary (claims)**
- https://thenewstack.io/sign-in-with-chatgpt/
- https://github.com/thedotmack/claude-mem/issues/1826
- https://x.com/bcherny/status/2040206441756471399
- https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access/

**ACP and crates**
- https://github.com/agentclientprotocol/claude-agent-acp
- https://github.com/agentclientprotocol/codex-acp
- https://crates.io/crates/agent-client-protocol

---

## Appendix A: the app-server probe

The probe is `docs/research/fixtures/codex-appserver-probe.mjs`. Run it as `node codex-appserver-probe.mjs <cwd> <outfile>` with `codex` on PATH. It spawns `codex app-server -c mcp_servers={}` with a clean environment and logs every line in both directions as `{"dir":"->|<-|stderr","t_ms":…,"msg":…}`.

- It redacts `email` and `accountId`.
- It auto-replies `{"decision":"accept"}` to `item/commandExecution/requestApproval` and `item/fileChange/requestApproval`.
- It interrupts turn 2 after the first `item/agentMessage/delta`.

The full fixture is `docs/research/fixtures/codex-appserver.jsonl`, 95 lines. `chatgptAccountId`, `installationId`, `accountId` and `email` were redacted after capture. Rate-limit reset-credit IDs and the user's local paths remain, so review the file before sharing it outside the team.

`tokio::process::Command::process_group` and `kill_on_drop` were checked against the tokio 1.53.3 source in the local cargo registry.
