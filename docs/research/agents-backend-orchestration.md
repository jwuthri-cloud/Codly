# API-key agent backend (Deep Agents sidecar) and orchestration patterns for Codly

Research date: **2026-10-25**. Scope: topic **A** (the API-key backend: LangChain Deep Agents plus OpenRouter or direct keys, run as a Tauri sidecar) and topic **B** (orchestration patterns, a cheap routing and decision layer, and a concrete orchestrator design for Codly's vertical slice).

**Evidence legend.** Each claim carries one of these tags:

- **[V-src]**: verified in source code cloned today (commit dates are given).
- **[V-docs]**: verified in official docs fetched today.
- **[V-local]**: verified by running it on this Mac today.
- **[V-reg]**: verified on the npm, PyPI or crates.io registry today.
- **[C]**: a vendor or project claim (marketing, a self-reported benchmark, or a blog). Not independently verified.
- **[I]**: my inference or recommendation, or a point I could not verify.

**Related reports in this folder:**

- `cli-others-acp.md`: ACP, Gemini, OpenCode, Cursor, Copilot and Pi. It also defines a normalized Rust `AgentEvent` enum. This report aligns its sidecar events to that enum.
- `landscape.md`.

---

## 0. TL;DR (decision-relevant)

1. **Deep Agents is alive and churning fast.**
   - Python `deepagents` is at **0.7.32** (2026-10-23): 33 releases in 90 days and about 30.6k GitHub stars.
   - JS `deepagents` is at **1.15.1** (2026-10-16): 18 releases in 90 days and about 1.6k stars. [V-reg][V-src]
   - Both had **breaking changes in summer 2026**:
     - The `write_todos` planning tool is now **opt-in** (Py 0.7.0 on 2026-07-29, JS 1.12.0).
     - The default system prompt is now **empty**.
     - `write_file` overwrites.
     - A recursive `delete` tool was added.
     - Python `FilesystemBackend` now defaults to `virtual_mode=True`. The **JS default is still `virtualMode: false`**. [V-src CHANGELOGs]
   - **Pin exact versions** and treat every upgrade as a migration.
2. **Use the JS package for the sidecar.** It now has a **typed, JSON-serializable event protocol**: `agent.streamEvents(input, { version: "v3" })` yields `{seq, method, params:{namespace, timestamp, node?, data}}`, on the `messages`, `tools`, `lifecycle`, `values`, `updates`, `custom` and other channels. Content-block deltas carry text, reasoning and tool-call args, and `message-finish` carries usage. This maps almost 1:1 onto Codly's normalized `AgentEvent`. [V-docs][V-src `@langchain/protocol@0.0.20`]
3. **A Bun-compiled sidecar works.** I built a 135-line PoC (deepagents 1.15.1, langchain 1.5.19, @langchain/openrouter 0.4.19):
   - It bundles **1,341 modules** in about 0.2 s.
   - The binary is **66.4 MB (arm64)** or **72.1 MB (x64)**, about 26.7 MB gzipped.
   - I compiled it but did not execute it, per the untrusted-code rule. [V-local]
   - Gotchas found:
     - **Never pass model strings.** JS `initChatModel` has no `openrouter` provider, and it uses a dynamic `import()` that a compiled binary cannot resolve. [V-src]
     - **`better-sqlite3` (used by `SqliteSaver`) is a native addon.** Port the 342-line saver to `bun:sqlite`. [V-src]
     - **langsmith peer range** is `>=0.7.1 <0.10.0`, but npm resolves 0.11.1. Pin `langsmith@0.9.x`. [V-src]
4. **Cost on OpenRouter is not free to get in streaming.**
   - OpenRouter always returns `usage.cost` in the last SSE chunk. [V-docs]
   - JS `ChatOpenRouter` maps only token counts. It also skips SSE chunks without `choices[0].delta`, so the usage-only final chunk may be dropped. [V-src]
   - Fix: tee `fetch` in the sidecar to capture `usage.cost` plus the generation `id`. Fall back to price-table estimates. Mark `cost_source`.
5. **For direct Anthropic or OpenAI keys, prefer "BYOK into the vendor CLI" over a new SDK.**
   - Claude Code runs with `ANTHROPIC_API_KEY`, or on OpenRouter via `ANTHROPIC_BASE_URL=https://openrouter.ai/api` plus `ANTHROPIC_AUTH_TOKEN` and an empty `ANTHROPIC_API_KEY`. [V-docs]
   - Codex supports custom `model_providers`, but **only `wire_api = "responses"`**; "chat" was removed. [V-src codex@2026-10-23]
   - Deep Agents is the vendor-neutral path, for any OpenRouter model, open-weight models, or a Codly-owned harness.
6. **Claude Agent SDK caveat.** It is proprietary-licensed and ships per-platform native Claude binaries. Its docs state: *"Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK."* [V-docs] It is therefore an API-key-only option for Codly. Get legal review of the "drive the user's own CLI subscription" model as a whole.
7. **Orchestration evidence converges on the shape "map-reduce-and-manage, single-threaded writes per area, a clean-context verifier, deterministic gates".**
   - Anthropic: multi-agent beat single-agent by 90.2% on research, at about 15x the tokens of chat. They warn that coding has fewer parallelizable subtasks. [V-docs]
   - Google, 2026-01: centralized coordination gave +80.9% on parallelizable tasks, but **every multi-agent variant lost 39–70% on sequential tasks**. Error amplification was 17.2x for independent agents versus 4.4x centralized. [V-docs]
   - Cognition, 2026-04-22: parallel writers "still see little real adoption". Writes should stay single-threaded, and reviewers start from a clean context. [V-docs]
   - A 2026-07 study of 33.6k agent PRs measured textual merge conflicts in **19.8%** of same-agent and **41.7%** of cross-agent co-active pairs. [V-docs arXiv 2607.04697]
8. **For Codly v1:**
   - An orchestrator (Rust) runs one planner call that returns a JSON DAG with acceptance criteria and **write scopes**.
   - At most 3 concurrent workers, each in its **own git worktree**, gated by dependencies and by **scope leases** (no two running tasks with overlapping write globs).
   - Codly itself runs the `verify` commands (tests, lint and typecheck) before any reviewer sees the work.
   - The reviewer is a **different vendor or model** with a clean context and gets only the diff plus the acceptance criteria.
   - A serialized merge queue merges into an integration branch, then runs the integration checks. The user merges the branch; Codly never pushes.
9. **Cheap decisions through subscriptions work, at about 3–4 s per call.** [V-local]
   - `claude -p --model haiku --output-format json --json-schema '<schema>'` returns a top-level **`structured_output`** object (Haiku 5.5, about 3.2 s, about 1.5k cache-write tokens with `--system-prompt`, `--tools ""` and `--setting-sources ""`).
   - `codex exec --json --ephemeral --ignore-user-config -s read-only -m gpt-6-luna --output-schema schema.json "<prompt>" < /dev/null` returns the JSON as the `agent_message` text (about 3.7 s, **15k input tokens**).
   - **Codex blocks on stdin unless it is `/dev/null`.**
   - **The ChatGPT login rejects `gpt-5.4-mini`.**
   - These calls burn the user's plan quota, so make about one decision call per task, not one per event.
10. **"TypeSafe Jev" is real.**
    - What it is: a "System One" decision model from TypeSafe AI that returns typed answers (`choice`, `score`, `noul`) with probabilities and confidence.
    - API and SDKs: `POST https://api.typesafe.ai/v1/systemone`, with SDKs `@typesafe-ai/sdk@0.7.0` and `typesafe-sdk@0.7.5`. [V-docs][V-reg]
    - Availability and price: also served on Vercel AI Gateway as `typesafe-ai/jev` at **$0.042 per 1M input tokens** with a 32k context, and on OpenRouter as `typesafe/jev-router`. [C][V-reg]
    - Calibration: "Calibrated" is a vendor claim. Community audits are mixed.
    - Use: it is a good optional L1 classifier in API-key mode. It is not usable through CLI subscriptions.
11. **Calibration approach.**
    - Don't trust verbalized confidence. In my n=1 probe, Claude said 0.80–0.85 and Codex said 0.99 for the same trivial routing question. The literature agrees that it is overconfident.
    - Use deterministic rules first, a cheap model second, and **empirical thresholds learned from Codly's own outcome log** (isotonic recalibration plus a selective-risk threshold).
    - High-risk operations are always human-gated by policy, never by a model.
12. **Recovery.**
    - Pre-assign session IDs: `claude --session-id <uuid>` and `gemini --session-id <uuid>` [V-local help]. Persist Codex's `thread_id` from the first `thread.started` line [V-local].
    - Write every raw stdout line to an append-only log before parsing.
    - On restart, kill orphaned process groups, then **resume** (`claude -p --resume`, `codex exec resume <id>`, or a Deep Agents thread with its checkpointer) with a "you were interrupted, check git status" continuation prompt.

---

## 1. Method and versions checked today

| Package | Version (date) | Source |
|---|---|---|
| `deepagents` (npm) | **1.15.1** (2026-10-16) | [V-reg] |
| `deepagents` (PyPI) | **0.7.32** (2026-10-23) | [V-reg] |
| `deepagents-code` (PyPI, the `dcode` CLI) | 0.2.1 (2026-10-23) | [V-reg] |
| `deepagents-cli` (PyPI, legacy name) | 0.3.0 (2026-08-24, superseded) | [V-reg] |
| `deepagents-acp` (npm / PyPI) | 0.1.36 / 0.0.13 | [V-reg] |
| `langchain` (npm / PyPI) | 1.5.19 / 1.4.7 | [V-reg] |
| `@langchain/core` | 1.3.0 | [V-reg] |
| `@langchain/langgraph` (npm) / `langgraph` (PyPI) | 1.5.2 / 1.2.17 | [V-reg] |
| `@langchain/openrouter` / `langchain-openrouter` | **0.4.19** / 0.2.9 | [V-reg] |
| `@langchain/openai` / `@langchain/anthropic` | 1.7.0 / 1.6.0 | [V-reg] |
| `@langchain/langgraph-checkpoint` / `-checkpoint-sqlite` | 1.2.1 / 1.1.0 | [V-reg] |
| `@anthropic-ai/claude-agent-sdk` / `claude-agent-sdk` (Py) | 0.3.310 / 0.2.174 | [V-reg] |
| `@openai/agents` / `openai-agents` (Py) | 0.21.0 / 0.24.0 | [V-reg] |
| `@openai/codex-sdk` / `@openai/codex` | 0.164.0 / 0.164.0 | [V-reg] |
| `@anthropic-ai/claude-code` | 2.1.310 (installed locally: 2.1.309) | [V-reg][V-local] |
| `@google/gemini-cli` | 0.65.0 (installed: 0.62.0) | [V-reg][V-local] |
| `@typesafe-ai/sdk` / `typesafe-sdk` (Py) | 0.7.0 / 0.7.5 | [V-reg] |
| Bun (local) | 1.3.14 | [V-local] |

Repositories cloned read-only:

- `langchain-ai/deepagents` at f26eb54 (2026-10-25)
- `langchain-ai/deepagentsjs` at ea12d8e (2026-10-25)
- `openai/codex` (2026-10-23, cloned by a sibling run)

Local probes are listed in §5.3 and §3.2. One methodology note: a crates.io version lookup I ran today sent the user's email address in the HTTP User-Agent header. That should not have happened. Nothing else was sent.

---

## 2. (A1) LangChain Deep Agents in October 2026

### 2.1 What it is now

`createDeepAgent` / `create_deep_agent` is a thin assembly over LangChain v1's `createAgent` (a LangGraph ReAct loop) plus a fixed middleware stack. In JS, the base stack and its order are [V-src `libs/deepagents/src/agent.ts`]:

1. `FilesystemMiddleware`: tools `ls`, `read_file`, `write_file`, `edit_file`, `delete`, `glob`, `grep`, `execute`. `execute` only works on a sandbox-protocol backend.
2. `SubAgentMiddleware`: the `task` tool, plus an auto-added `general-purpose` subagent.
3. `SummarizationMiddleware`: the deepagents version, which offloads history to the backend.
4. `PatchToolCallsMiddleware`: repairs dangling tool calls after a cancel or interrupt.
5. `AsyncSubAgentMiddleware`, if async subagents are configured: `start_async_task`, `check_async_task`, `update_async_task`, `cancel_async_task`, `list_async_tasks`.
6. User middleware.
7. The tail: harness-profile middleware, Skills, Anthropic or Bedrock prompt caching, Memory, `humanInTheLoopMiddleware` (if `interruptOn` is set), and `UnsupportedContentMiddleware`.

The agent is compiled with `recursionLimit: 10_000`. Custom tool names that collide with built-ins throw `ConfigurationError("TOOL_NAME_COLLISION")`. [V-src]

### 2.2 JS API (deepagents 1.15.1)

```ts
// CreateDeepAgentParams (abridged from src/types.ts) [V-src]
createDeepAgent({
  model?: BaseLanguageModel | string,       // default "anthropic:claude-sonnet-4-6"
  tools?: StructuredTool[],
  systemPrompt?: string | SystemMessage,     // default prompt is now EMPTY (1.12.0)
  middleware?: AgentMiddleware[],            // e.g. [todoListMiddleware()] to get write_todos back
  subagents?: (SubAgent | CompiledSubAgent | AsyncSubAgent)[],
  responseFormat?: ZodSchema | toolStrategy(...) | providerStrategy(...),
  contextSchema?, stateSchema?,
  checkpointer?: BaseCheckpointSaver | boolean,   // REQUIRED for interrupts/resume
  store?: BaseStore,                              // for StoreBackend
  backend?: Backend | ((cfg:{state, store}) => Backend),  // default StateBackend
  interruptOn?: Record<string, boolean | InterruptOnConfig>,
  memory?: string[],        // AGENTS.md paths, loaded into the system prompt
  skills?: string[],        // skill source dirs (SKILL.md), progressive disclosure
  permissions?: FilesystemPermission[],  // {operations:["read"|"write"], paths:[globs], mode?:"allow"|"deny"}
  name?: string,
  streamTransformers?: (() => StreamTransformer)[],
});
```

The `SubAgent` spec [V-src `middleware/subagents.ts`]:

```ts
{ name, description,                // description drives the coordinator's choice
  systemPrompt?, tools?,            // tools default to the parent's tools (fixed in 1.13.x)
  model?,                           // per-subagent model (instance recommended, see §3.4)
  middleware?, interruptOn?,        // inherits the parent's interruptOn unless overridden
  skills?, permissions?,
  responseFormat?,                  // structured result -> JSON ToolMessage to the parent
  mode?: "isolated" | "fork" }      // "fork" (experimental): inherit parent history (cache-friendly)
```

The `task` tool takes `{ description: string, subagent_type: string }`. The subagent runs **isolated** by default: it sees only `description` and returns its final message, or its structured response, as the tool result. [V-src]

### 2.3 Python API (deepagents 0.7.32)

```python
create_deep_agent(model=None, tools=None, *, system_prompt=None, middleware=(), subagents=None,
                  skills=None, memory=None, permissions=None, backend=None, interrupt_on=None,
                  response_format=None, state_schema=None, context_schema=None,
                  checkpointer=None, store=None, debug=False, name=None, cache=None)
```

[V-src `deepagents/graph.py`]. Python-only extras:

- `FilesystemPermission(mode="interrupt")`, which auto-installs HITL. JS supports only `allow|deny`.
- `RubricMiddleware`.
- Provider profiles for OpenAI, OpenRouter and NVIDIA. The OpenRouter profile injects attribution headers and `provider={"ignore":["azure"]}` to avoid a stateless `/responses` reasoning-replay bug.
- Remote sandbox partners: Daytona, Modal, Runloop, Vercel, QuickJS.
- `model=None` is deprecated, to be removed in 1.0. [V-src]

### 2.4 Breaking changes in 2026 that matter for Codly [V-src CHANGELOGs]

| Change | Python | JS |
|---|---|---|
| `TodoListMiddleware` / `write_todos` removed from defaults (opt-in) | 0.7.0 (2026-07-29) | 1.12.0 (2026-07-29) |
| Default system prompt now empty ("lean prompting"); `BASE_AGENT_PROMPT` deprecated | 0.7.0 | 1.12.0 |
| `write_file` creates or **replaces** (no "file exists" guard) | 0.7.0 | 1.12.0 |
| New destructive recursive `delete` tool; counted as a write for permissions | 0.7.0 | present in `FILESYSTEM_TOOL_NAMES` |
| `FilesystemBackend` / `LocalShellBackend` default `virtual_mode=True` | 0.7.0 | **still `false` by default**: set `virtualMode: true` explicitly |
| `read_file` output: `@@ lines A-B of T @@` header instead of a line-number gutter | n/a | 1.13.5 |
| Failed tool results marked `status:"error"` | n/a | 1.15.0 |
| `CompositeBackend` matches on path-segment boundaries (`/foo` no longer captures `/foobar.txt`) | n/a | 1.15.0 |

**Implication [I]:**

- Pin `deepagents@1.15.1` and the matching `langchain`/`@langchain/*` versions in the sidecar's lockfile.
- Add a contract test that replays a recorded v3 stream (see §3.8) on every upgrade.
- Codly must supply its own system prompt, because the default is now empty.

### 2.5 Planning (`write_todos`)

Planning is opt-in: `middleware: [todoListMiddleware()]`, imported from `langchain`. The tool schema is `{ todos: [{ content: string, status: "pending"|"in_progress"|"completed" }] }`. It adds `todos` to state, and it errors if called in parallel within one model turn. [V-src `langchain/dist/agents/middleware/todoListMiddleware.js`] Add it per subagent too, if wanted. The built-in `openai-codex` harness profile adds it automatically. [V-src]

**For Codly [I]:** turn it on for the root agent. Map each `write_todos` call to `AgentEvent::Plan`. The Kanban and Living World views then get a live checklist per agent.

### 2.6 Subagents

- **Sync subagents** go through `task`. The default `general-purpose` subagent can be disabled through a harness profile (`generalPurposeSubagent.enabled=false`). [V-src]
- **Parallel `task` calls** run concurrently, as LangGraph `Send` tasks. Shared custom state keys need reducers, otherwise you get `InvalidUpdateError`. 1.13.4 fixed call-limit counters leaking across the boundary. [V-src]
- **`CompiledSubAgent`** (`{name, description, runnable}`) lets Codly plug in any LangGraph graph.
- **`AsyncSubAgent`** (`{name, description, graphId, url?, headers?}`) runs a background task on a LangGraph server. It is not needed for a local desktop app. [V-src]
- **Interrupt inheritance**: declarative subagents inherit the top-level `interruptOn` unless they override it. Compiled and async subagents don't. [V-src docstring]

### 2.7 Backends [V-src `src/backends/*`]

| Backend | Use | Notes |
|---|---|---|
| `StateBackend` (default) | ephemeral virtual FS in graph state | Files live in checkpoints. Large repos bloat state. **Not for Codly workers.** |
| `FilesystemBackend({rootDir, virtualMode, maxFileSizeMb=10})` | real disk | `grep` shells out to `rg` (30 s timeout). Set `virtualMode:true` to anchor paths and reject `..` and symlink escapes. |
| `LocalShellBackend({rootDir, virtualMode=false, timeout=120 (s), maxOutputBytes=100_000, inheritEnv=false, env?})` | real disk **plus `execute`** | Runs commands in `rootDir`, with stdout and stderr combined and truncated. **No sandbox.** |
| `StoreBackend({namespace})` | LangGraph `BaseStore` | JS ships only `InMemoryStore` in-box. A persistent store needs Postgres etc. |
| `CompositeBackend(default, {"/memories/": other})` | route by path prefix | Prefix stripped and re-added. Sandbox default is adapted. |
| `BaseSandbox`, `LangSmithSandbox`, provider packages (`@langchain/daytona`, `@langchain/modal`, `@langchain/deno`, `@langchain/node-vfs`, `@langchain/quickjs`) | remote or isolated execution | |

**For Codly [I]:**

```ts
new CompositeBackend(
  new LocalShellBackend({ rootDir: worktreePath, virtualMode: true, timeout: 300, inheritEnv: false,
                          env: { PATH: safePath, HOME: worktreeHome } }),
  { "/memories/": new FilesystemBackend({ rootDir: `${appData}/memories/${workspaceId}`, virtualMode: true }) },
)
```

This gives persistent per-workspace memory without a store DB.

### 2.8 Permissions

`permissions: [{operations:["write"], paths:["/src/auth/**"]}, {operations:["write"], paths:["/**"], mode:"deny"}]`. The first match wins, and the default is allow. **These rules apply only to the built-in FS tools, not to `execute`.** A shell command can write anywhere `rootDir` allows. [V-src docs comment] Codly can enforce a task's write scope this way for the FS tools. It still needs post-hoc diff checks (§6.7) and HITL on `execute`.

### 2.9 Summarization and context management [V-src `middleware/summarization.ts`]

- **Trigger and keep defaults.** If the model profile has `maxInputTokens`, summarization triggers at **0.85** of context and keeps **0.10**. Otherwise it triggers at **170,000 tokens** and keeps the **last 6 messages**.
- **Tool-argument truncation** has its own trigger and keep settings.
- **Offload path.** Old messages are offloaded as markdown to `/conversation_history/{thread_id}.md` on the backend.
- **Tool-result offload.** Large tool results and binary content are offloaded to files (`offloadBinaryContent`, blob offload).

**For Codly [I]:** route `/conversation_history/` to an app-data directory, not the worktree, so it never shows up in diffs.

### 2.10 Memory and long-term store

- `memory: ["/AGENTS.md", ...]`: these files are read through the backend at startup and injected into the system prompt.
- Agent-written long-term memory: route `/memories/` to a persistent backend.
- `skills: ["/skills/"]`: SKILL.md files with progressive disclosure. 1.15 adds `pinnedSkills` and skill-scoped tools. [V-src]
- **Codly [I]:** point `memory` at the repo's `AGENTS.md` / `CLAUDE.md` so the sidecar sees the same project rules as the CLI agents.

### 2.11 Human-in-the-loop [V-docs `deepagents/human-in-the-loop`][V-src]

- **Configuration:** `interruptOn: { execute: true, delete: true, write_file: { allowedDecisions: ["approve","reject"] } }`.
  - `true` means approve, edit, reject or respond.
  - The `when` predicate on `InterruptOnConfig` exists in langchain JS 1.5.19 for conditional interrupts, for example "execute only when the command is not on the allowlist". [V-src `hitl.d.ts`]
  - **A checkpointer and a `thread_id` are required.**
- **Interrupt payload:** `{ actionRequests: [{name, args, description?}], reviewConfigs: [{actionName, allowedDecisions}] }`. All tool calls in one turn are **batched** into one interrupt.
- **Resume:** `new Command({ resume: { decisions: [{type:"approve"} | {type:"edit", editedAction:{name,args}} | {type:"reject", message} | {type:"respond", ...}] } })` with the same `thread_id`. With **several pending interrupts** (parallel subagents), map each interrupt id to its resume value.
- **In-process v3 streaming** exposes pauses through `stream.interrupted` / `stream.interrupts` (`[{interruptId, payload}]`) *after* iteration. The raw `input.requested` method belongs to the LangGraph **server** wire protocol. In-process, interrupts are detected from `values` chunks carrying `__interrupt__`. [V-src `@langchain/langgraph/dist/stream/mux.js`]
- **Legacy streaming** (`streamMode:["messages","updates"]`): interrupts appear as `updates` chunks with `__interrupt__`. [V-docs]

### 2.12 Streaming

There are two APIs, and both work in 1.15.1.

**(a) Legacy LangGraph stream modes** [V-src `examples/streaming/*.ts`]:

```ts
for await (const [namespace, mode, data] of await agent.stream(input,
    { streamMode: ["updates", "messages", "custom"], subgraphs: true })) { ... }
// namespace [] = root; a subagent lives under a "tools:<id>" segment
// messages -> [messageChunk, metadata]; custom -> whatever tools write via config.writer?.(obj)
```

**(b) v3 event protocol (recommended)** [V-docs `langgraph/event-streaming`, `deepagents/event-streaming`][V-src `@langchain/protocol/protocol.ts`]:

```ts
interface ProtocolEvent {
  seq: number;                       // strictly increasing within a run
  method: "messages"|"tools"|"lifecycle"|"values"|"updates"|"custom"|"checkpoints"|"tasks"|"input"|`custom:${string}`;
  params: { namespace: string[];     // ["<name>:<runtime_id>", ...]; [] is root
            timestamp: number; node?: string; data: unknown };
}
// messages.data.event: message-start | content-block-start | content-block-delta | content-block-finish
//                      | message-finish{usage?:{input_tokens,output_tokens,total_tokens,
//                        input_token_details{cache_read,cache_creation}, output_token_details{reasoning}}} | error
//   content-block-delta.delta.type: text-delta{text} | reasoning-delta{reasoning} | tool-call-delta{args}
//                                   | data-delta{data} | block-delta{fields}
// tools.data.event: tool-started{tool_call_id, tool_name, input} | tool-output-delta{tool_call_id, delta}
//                   | tool-finished{tool_call_id, output} | tool-error{tool_call_id, message, code?}
// lifecycle.data: {event: started|running|completed|failed|interrupted, graph_name?, cause?, error?}
```

Typed projections are also available:

- `stream.messages`, `stream.toolCalls`, `stream.values`, `stream.subgraphs`, `stream.output`, `stream.interrupts`.
- Deep Agents' **`stream.subagents`**: one handle per `task` call, with `name` (= `subagent_type`), `taskInput`, `messages`, `toolCalls`, `subagents` (nested) and `output`. [V-docs]

**Subagent attribution in raw events** [V-src `langchain/dist/agents/transformers/subagent.js`]: *"A tool that invokes a subagent emits its `tool-started` at the tools-node namespace (`tools:<task_id>`) where the subagent then roots."* So when you see `tool-started{tool_name:"task", tool_call_id}` at namespace N, every later event whose namespace has prefix N belongs to that subagent. The PoC in §3.8 uses exactly this.

### 2.13 Models

- **Model strings** use `provider:model` format, for example `"anthropic:claude-sonnet-4-6"` or `"openai:gpt-5.5"`. `openai:` strings default to the Responses API through a provider profile. [V-docs `deepagents/models`]
- **JS `initChatModel` providers** in langchain 1.5.19: `openai, anthropic, azure_openai, langsmith, cohere, google(-vertexai/-genai), ollama, mistral(ai), groq, bedrock/aws, deepseek, xai, cerebras, fireworks, together, perplexity`. **There is no `openrouter`.** [V-src `universal.js`] The docs' eval table rows like `openrouter:z-ai/glm-5.1` are Python-style. In JS, construct `new ChatOpenRouter(...)`.
- **Eval table** ("Deep Agents eval suite", [C], LangChain's own suite) as of the docs fetch:
  - `openrouter:z-ai/glm-5.1`: 89% overall
  - `google_genai:gemini-3.6-flash`: 82%
  - `openrouter:deepseek/deepseek-v4-flash`: 81%
  - `openai:gpt-5.5` and `anthropic:claude-opus-4-7`: 80%
  - The docs say passing these evals "is necessary but not sufficient" for long, complex tasks.
  - **Not comparable to SWE-bench.** [V-docs]

### 2.14 CLIs around Deep Agents

- **`dcode` / `deepagents-code` 0.2.1** (Python; install through `curl -LsSf https://langch.in/dcode | bash`). A Textual TUI coding agent with resume, web search, remote sandboxes, memory, skills, HITL and a **headless mode**. [V-src README] I did not enumerate its headless flags [I]. It could become one more CLI adapter, but it is Python and needs an install, so it is not the default backend.
- **`deepagents-acp`**: an ACP server (`npx deepagents-acp --model ... --workspace ...`).
  - It emits ACP `tool_call`, `tool_call_update`, `plan` and `available_commands_update`. It does **not** expose the subagent tree or usage. [V-src `libs/acp/src/server.ts`]
  - Its SDK pin is `@agentclientprotocol/sdk ^1.1.0`, while 2.0.2 is current. [V-reg]
  - **Not rich enough for the Living World.** Use it only as a compatibility fallback.

### 2.15 JS vs Python maturity

| | JS `deepagents` 1.15.1 | Python `deepagents` 0.7.32 |
|---|---|---|
| Adoption | ~1.6k stars, 118 open issues | ~30.6k stars, 193 open issues [V-reg gh] |
| Cadence (90 days) | 18 releases | 33 releases |
| Feature parity | "1:1 compatibility" claimed [C]; lags in places (no `interrupt` permission mode, no rubric, fewer provider profiles) | Reference implementation, more features, plus the `dcode` CLI and evals |
| Streaming | v3 protocol with typed subagent projection | v3 too (`stream_events(version="v3")`) |
| Packaging for Tauri | **Single 66–72 MB Bun binary** [V-local] | PyInstaller or embedded Python (heavier, slower start) [I] |
| Sandboxes | daytona, deno, modal, node-vfs, quickjs | daytona, modal, runloop, vercel, quickjs |

**Verdict [I]:** use JS for the sidecar (one language with the frontend, a single-binary distribution, the v3 protocol). Watch the Python changelog as the leading indicator of upcoming JS changes.

---

## 3. (A2) Running the API-key backend as a Tauri sidecar

### 3.1 Architecture [I]

```
Tauri app (Rust core)
 ├─ AgentSupervisor (tokio): spawns ALL agent processes, owns stdin/stdout, writes raw lines to
 │    runs/<run_id>/raw.ndjson before parsing, normalizes to AgentEvent, persists to SQLite (WAL)
 │    ├─ ClaudeAdapter   (claude -p stream-json + control protocol)      ─┐
 │    ├─ CodexAdapter    (codex app-server / exec --json)                  │ same AgentEvent
 │    ├─ AcpAdapter      (gemini --acp, opencode acp, cursor, copilot…)   │ enum, same Run/Event
 │    └─ DeepAgentsAdapter (codly-agent sidecar, NDJSON JSON-RPC)        ─┘ tables
 ├─ Orchestrator (planner, scheduler, gates, merge queue)  — §6
 └─ IPC → TS frontend (Workspace view + Living World view are both projections of the event log)
```

The sidecar is **one long-lived process with many runs** (multiplexed by `run_id`), spawned lazily on first API-key use.

**Secrets [I]:** keys live in the macOS Keychain (e.g. `security-framework` / `keyring` crates) and are passed **only as env vars at spawn**. They never go on the wire, into logs, or into SQLite.

### 3.2 Packaging (measured today) [V-local]

```bash
cd sidecar && bun install --frozen-lockfile --ignore-scripts
bun build --compile --minify --bytecode --target=bun-darwin-arm64 ./src/main.ts \
  --outfile ../src-tauri/binaries/codly-agent-aarch64-apple-darwin
bun build --compile --minify --bytecode --target=bun-darwin-x64   ./src/main.ts \
  --outfile ../src-tauri/binaries/codly-agent-x86_64-apple-darwin
```

**Measured results:**

- 1,341 modules bundled.
- The plain JS bundle is 2.94 MB.
- The compiled binary is **66.4 MB (arm64)** and **72.1 MB (x64)**, about 26.7 MB gzipped (arm64).
- Compile time is about 0.1–0.2 s, plus a one-time 26 MB download of the x64 Bun runtime for cross-compiles.
- `--bytecode` is recommended by Bun docs for startup [V-docs]. I did not measure startup, since the binary was not executed.

**Signing [V-docs bun executables]:** Bun says to codesign with an entitlements plist containing `com.apple.security.cs.allow-jit` (JSC JIT) under the hardened runtime. Tauri's bundler signs `externalBin` entries with the app identity. Add the entitlement to the app's `Entitlements.plist` [I: verify notarization end to end].

**Universal builds [I]:** Tauri expects `-universal-apple-darwin` for universal targets. `lipo` of the two binaries is about 138 MB, so ship per-architecture DMGs instead.

### 3.3 Tauri wiring [V-docs `v2.tauri.app/develop/sidecar`]

```jsonc
// src-tauri/tauri.conf.json
{ "bundle": { "externalBin": ["binaries/codly-agent"] } }   // file must be binaries/codly-agent-$TARGET_TRIPLE
```

```rust
// Rust side (tauri-plugin-shell 2.5.0) — capabilities are only needed if the *webview* spawns it
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
let (mut rx, mut child) = app.shell().sidecar("codly-agent")?   // file name only, not the path
    .env("OPENROUTER_API_KEY", key)                              // from Keychain
    .spawn()?;
child.write(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")?;
while let Some(ev) = rx.recv().await {
    if let CommandEvent::Stdout(line) = ev { supervisor.ingest(run_ids, &line).await?; }
}
```

Get the target triple with `rustc --print host-tuple` (Rust ≥1.84). [V-docs]

**Recommendation [I]:** resolve the sidecar path once with the shell plugin, but spawn it with **`tokio::process::Command` + `process-wrap`** (own process group, kill-on-drop, `kill_on_drop(true)`), the same code path as the CLI adapters. One supervisor implementation is simpler than two.

### 3.4 Gotchas found while building the PoC [V-src / V-local]

1. **Model strings break in a compiled binary.** `initChatModel` does `await import(config.package)` with a non-literal argument, which the bundler can't follow. Pass **instances** (`new ChatAnthropic`, `new ChatOpenAI`, `new ChatOpenRouter`) for the root agent **and every subagent**.
2. **There is no `openrouter:` model string in JS.** Use `@langchain/openrouter`. It talks directly to `${baseURL}/chat/completions` with `fetch` and SSE (default `baseURL` `https://openrouter.ai/api/v1`).
   - Params: `model`, `apiKey` (env `OPENROUTER_API_KEY`), `siteUrl` (→ `HTTP-Referer`), `siteName` (→ `X-Title`), `appCategories`, `models` + `route:"fallback"`, `provider` preferences, `transforms`, `plugins`, `sessionId`, `trace`, `streamUsage` (default true), `modelKwargs`, and the usual sampling params.
   - Reasoning is preserved in `additional_kwargs.reasoning_content` / `reasoning_details`.
3. **`@langchain/langgraph-checkpoint-sqlite` depends on `better-sqlite3`,** a native addon (not built with `--ignore-scripts`, risky under `bun --compile`). Port its 342-line `SqliteSaver` to `bun:sqlite`. The API is near-identical, and only 5 abstract methods are needed: `getTuple`, `list`, `put`, `putWrites`, `deleteThread`. Store checkpoints in `appData/agents-checkpoints.sqlite`, separate from Codly's main DB.
4. **The langsmith peer range is `>=0.7.1 <0.10.0`,** but a fresh install pulled 0.11.1. Pin `langsmith@0.9.0` and set `LANGSMITH_TRACING=false` unless the user opts in.
5. **Prompts:** the default system prompt is empty since 1.12. Codly must supply role prompts (planner, worker, reviewer).

### 3.5 Providers and cost accounting

| Provider | Class | Cost signal |
|---|---|---|
| OpenRouter | `ChatOpenRouter({model:"z-ai/glm-5.1", siteName:"Codly", siteUrl:"https://codly.app", provider:{ignore:["azure"]}})` | OpenRouter returns `usage.cost` (credits), `prompt_tokens_details.cached_tokens`, `cache_write_tokens` and `completion_tokens_details.reasoning_tokens` in **every** response, and in the **last SSE chunk** when streaming. `usage:{include:true}` is deprecated and a no-op. [V-docs usage-accounting] **But** `ChatOpenRouter` maps only token counts into `usage_metadata` and skips chunks without `choices[0].delta`. [V-src] |
| Anthropic | `ChatAnthropic({model, streaming:true})` | Tokens incl. cache read and write, so cost = tokens × Codly's price table. |
| OpenAI | `ChatOpenAI({model, useResponsesApi:true})` | Tokens incl. reasoning, cost from the price table. |
| Any OpenAI-compatible | `ChatOpenAI({configuration:{baseURL, apiKey}})` | Tokens only. |

**OpenRouter cost capture [I]:** wrap `globalThis.fetch` in the sidecar. For `POST https://openrouter.ai/api/v1/chat/completions`, `tee()` the response body, parse SSE `data:` lines, and keep the last object that has `usage`. Then emit `Usage{cost_usd: usage.cost, source:"provider"}` keyed by the generation `id`. Reconcile asynchronously with OpenRouter's "get generation" endpoint using that `id` if the chunk is missing [V-docs mentions it; exact path not verified]. Otherwise fall back to `pricing.prompt/completion/input_cache_read` from `GET https://openrouter.ai/api/v1/models` (works unauthenticated, [V-local]) with `source:"estimated"`.

### 3.6 Sidecar protocol (Codly Agent Protocol v1) [I]

**Framing:** NDJSON, **JSON-RPC 2.0 shaped**, like ACP, LSP and MCP. Codly's Rust JSON-RPC transport (planned for ACP in `cli-others-acp.md` R2) can then drive the sidecar too.

**Commands (Codly → sidecar):**

| method | params | result |
|---|---|---|
| `initialize` | `{protocol:1}` | `{protocol, runtime, versions:{deepagents, langchain}}` |
| `run/start` | `{run_id, thread_id, cwd, model:{provider, model}, system_prompt, prompt, subagents:[{name, description, systemPrompt, model?}], interrupt_on, permissions?, response_schema?}` | `{accepted:true}` |
| `run/resume` | `{run_id, decisions:[...]}` or `{run_id, by_interrupt:{<interrupt_id>:{decisions}}}` | `{accepted:true}` |
| `run/input` | `{run_id, text}` (follow-up user turn on the same thread) | `{accepted:true}` |
| `run/cancel` | `{run_id}` | `{accepted:true}` (abort signal; PatchToolCalls repairs history) |
| `decide` | `{schema, prompt, model}` (one-shot structured call, §5) | `{output, usage}` |
| `shutdown` | `{}` | then exit |

**Notifications (sidecar → Codly):** `{"jsonrpc":"2.0","method":"codly/event","params":{"run_id","seq","ts","ns":[...],"event":{...}}}`. Here `event` is **serde-compatible with the Rust `AgentEvent`** proposed in `cli-others-acp.md` §11 (tag `type`, snake_case). The supervisor deserializes it directly:

| `event.type` | Fields | AgentEvent variant |
|---|---|---|
| `run_state` | `state: running` | (Run status) |
| `message_delta` | `text` | `MessageDelta` |
| `thought_delta` | `text` | `ThoughtDelta` |
| `tool_started` | `id, name, kind (read/edit/delete/search/execute/think/other), raw_input` | `ToolStarted` |
| `tool_updated` | `id, status (completed/failed), raw_output` (≤8 KB; the full output goes to an artifact file) | `ToolUpdated` |
| `plan` | `entries:[{content,status}]` | `Plan` |
| `usage` | `tokens{input_tokens,output_tokens,input_token_details,output_token_details}, cost_usd?, source: provider/estimated` | `Usage` |
| `permission_asked` | `interrupt_id, request{actionRequests, reviewConfigs}` | `PermissionAsked` |
| `subagent_spawned` | `cause_tool_call_id, title (subagent_type), task (description)`; `ns` = subagent root | `SubagentSpawned` |
| `subagent_state` | `state: completed/failed` | `SubagentState` |
| `turn_ended` | `stop: end_turn/interrupted/cancelled/error, error?` | `TurnEnded` |

**Rules:**

- `seq` is monotonic per process. Codly persists `(run_id, seq)` for idempotent ingestion.
- stdout carries **only** protocol lines; logs go to stderr.
- The sidecar exits on stdin EOF, so it can never be orphaned.

### 3.7 Mapping CLI outputs to the same events (summary, [I])

The details are in the sibling CLI reports. The point is that the **Deep Agents adapter is the easiest of all**, because the v3 protocol already has explicit tool-call ids, subagent namespaces and usage. Codly's own derived events are adapter-independent:

- `FileChanged`, from Rust `notify` on worktrees plus `git diff --name-only`.
- `TaskState`, from the orchestrator.
- `GateResult`, from verify commands.

Never derive "what the agent is doing" from model prose, only from tool events.

### 3.8 Reference implementation (PoC, compiled with Bun 1.3.14; not executed)

```ts
// sidecar/src/main.ts — codly-agent (deepagents@1.15.1, langchain@1.5.19, @langchain/openrouter@0.4.19)
import { createDeepAgent, LocalShellBackend } from "deepagents";
import { todoListMiddleware } from "langchain";
import { ChatAnthropic } from "@langchain/anthropic";
import { ChatOpenAI } from "@langchain/openai";
import { ChatOpenRouter } from "@langchain/openrouter";
import { MemorySaver, Command } from "@langchain/langgraph";
import * as readline from "node:readline";

let seq = 0;
const send = (o: object) => process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...o }) + "\n");
const emit = (run_id: string, ns: string[], event: { type: string; [k: string]: unknown }) =>
  send({ method: "codly/event", params: { run_id, seq: ++seq, ts: Date.now(), ns, event } });

const checkpointer = new MemorySaver(); // production: bun:sqlite port of SqliteSaver (§3.4)
const runs = new Map<string, { agent: any; thread_id: string; abort: AbortController }>();

function makeModel(p: { provider: "anthropic" | "openai" | "openrouter"; model: string }) {
  switch (p.provider) { // instances only — see §3.4 (1)
    case "anthropic": return new ChatAnthropic({ model: p.model, streaming: true });
    case "openai": return new ChatOpenAI({ model: p.model, streaming: true, useResponsesApi: true });
    case "openrouter": return new ChatOpenRouter({ model: p.model, siteName: "Codly" } as any);
  }
}
const TOOL_KIND: Record<string, string> = { ls: "read", read_file: "read", glob: "search", grep: "search",
  write_file: "edit", edit_file: "edit", delete: "delete", execute: "execute", write_todos: "think", task: "other" };

async function pump(run_id: string, input: unknown) {
  const r = runs.get(run_id)!;
  const subagentByCall = new Map<string, string[]>();
  emit(run_id, [], { type: "run_state", state: "running" });
  try {
    const stream: any = await r.agent.streamEvents(input,
      { version: "v3", configurable: { thread_id: r.thread_id }, signal: r.abort.signal });
    for await (const ev of stream as AsyncIterable<any>) {
      const ns: string[] = ev.params.namespace ?? []; const d = ev.params.data;
      if (ev.method === "messages") {
        if (d.event === "content-block-delta" && d.delta?.type === "text-delta")
          emit(run_id, ns, { type: "message_delta", text: d.delta.text });
        else if (d.event === "content-block-delta" && d.delta?.type === "reasoning-delta")
          emit(run_id, ns, { type: "thought_delta", text: d.delta.reasoning });
        else if (d.event === "message-finish" && d.usage)
          emit(run_id, ns, { type: "usage", tokens: d.usage, cost_usd: null, source: "estimated" });
      } else if (ev.method === "tools") {
        if (d.event === "tool-started") {
          emit(run_id, ns, { type: "tool_started", id: d.tool_call_id, name: d.tool_name,
            kind: TOOL_KIND[d.tool_name] ?? "other", raw_input: d.input });
          if (d.tool_name === "write_todos") emit(run_id, ns, { type: "plan", entries: d.input?.todos ?? [] });
          if (d.tool_name === "task") { // subagent roots at this namespace (§2.12)
            subagentByCall.set(d.tool_call_id, ns);
            emit(run_id, ns, { type: "subagent_spawned", cause_tool_call_id: d.tool_call_id,
              title: d.input?.subagent_type, task: d.input?.description });
          }
        } else if (d.event === "tool-finished" || d.event === "tool-error") {
          const failed = d.event === "tool-error";
          emit(run_id, ns, { type: "tool_updated", id: d.tool_call_id, status: failed ? "failed" : "completed",
            raw_output: failed ? d.message : String(d.output ?? "").slice(0, 8000) });
          const sub = subagentByCall.get(d.tool_call_id);
          if (sub) emit(run_id, sub, { type: "subagent_state", state: failed ? "failed" : "completed" });
        }
      }
    }
    for (const it of stream.interrupted ? stream.interrupts : [])   // in-process HITL (§2.11)
      emit(run_id, [], { type: "permission_asked", interrupt_id: it.interruptId, request: it.payload });
    emit(run_id, [], { type: "turn_ended", stop: stream.interrupted ? "interrupted" : "end_turn" });
  } catch (e: any) {
    emit(run_id, [], { type: "turn_ended", stop: r.abort.signal.aborted ? "cancelled" : "error", error: String(e?.message ?? e) });
  }
}

async function handle(method: string, p: any) {
  switch (method) {
    case "initialize": return { protocol: 1, runtime: typeof Bun !== "undefined" ? `bun ${Bun.version}` : process.version };
    case "run/start": {
      const agent = createDeepAgent({
        model: makeModel(p.model), systemPrompt: p.system_prompt,
        backend: new LocalShellBackend({ rootDir: p.cwd, virtualMode: true, timeout: 120, inheritEnv: false }),
        middleware: [todoListMiddleware()],
        subagents: (p.subagents ?? []).map((s: any) => ({ ...s, model: s.model ? makeModel(s.model) : undefined })),
        interruptOn: p.interrupt_on ?? { execute: true, delete: true },
        checkpointer,
      });
      runs.set(p.run_id, { agent, thread_id: p.thread_id, abort: new AbortController() });
      void pump(p.run_id, { messages: [{ role: "user", content: p.prompt }] });
      return { accepted: true };
    }
    case "run/resume": {
      const r = runs.get(p.run_id); if (!r) throw Object.assign(new Error("no such run"), { code: -32002 });
      r.abort = new AbortController();
      void pump(p.run_id, new Command({ resume: p.by_interrupt ?? { decisions: p.decisions } }));
      return { accepted: true };
    }
    case "run/cancel": runs.get(p.run_id)?.abort.abort(); return { accepted: true };
    default: throw Object.assign(new Error(`method not found: ${method}`), { code: -32601 });
  }
}
const rl = readline.createInterface({ input: process.stdin });
rl.on("line", async (line) => {
  if (!line.trim()) return;
  let m: any; try { m = JSON.parse(line); } catch { return send({ id: null, error: { code: -32700, message: "parse error" } }); }
  try { send({ id: m.id, result: await handle(m.method, m.params ?? {}) }); }
  catch (e: any) { send({ id: m.id, error: { code: e?.code ?? -32603, message: String(e?.message ?? e) } }); }
});
rl.on("close", () => process.exit(0));
```

**Not verified [I]:**

- Runtime behavior (not executed).
- That `tool-started` for nested tools carries the subagent prefix in every provider path.
- The exact `input` shape of `task` across versions.

**Next step:** a recorded-fixture contract test. Run it once with a cheap OpenRouter model, save the NDJSON under `docs/research/fixtures/deepagents-v3.ndjson`, and assert on the mapping.

### 3.9 Persistence and resume inside the sidecar [I]

- `thread_id` = Codly `run.session_id` (chosen by Codly, a UUIDv7).
- With the bun:sqlite saver, a sidecar restart can **resume a paused HITL run**: `run/resume` rebuilds the agent with the same config and passes `Command({resume})` on the same `thread_id`.
- It can also **continue after a crash**: `run/input` with "Codly restarted; check `git status` and continue".
- Keep `MemorySaver` only for tests.

### 3.10 Security [I]

- `LocalShellBackend.execute` is an **unsandboxed** `spawn(..., {shell:true, cwd:rootDir})`. Mitigations, in order:
  1. Always `interruptOn.execute` with a `when` predicate that auto-allows a per-workspace allowlist (`npm test`, `cargo test`, `pnpm lint`, …).
  2. `inheritEnv:false` and an explicit minimal `env`.
  3. `rootDir` = the task worktree, with `virtualMode:true`.
  4. Optional macOS Seatbelt wrapping (`sandbox-exec -p <profile>`), the approach Codex and Claude Code take. The API is deprecated but functional [I: verify on macOS 26].
- Never give the sidecar the user's CLI credentials. It only gets the provider key(s) it needs.

### 3.11 Alternatives compared

| Option | Models | Harness quality | Events for Codly | Packaging / licence | Verdict |
|---|---|---|---|---|---|
| **Deep Agents JS** (this report) | Any: OpenRouter, Anthropic, OpenAI, Google, Ollama… | Generic ReAct + FS/shell + subagents; vendor eval only [C] | **v3 protocol**: deltas, tools, subagent namespaces, usage, interrupts | 66–72 MB Bun binary; MIT | **The vendor-neutral API-key backend.** |
| **Claude Agent SDK** (TS 0.3.310 / Py 0.2.174) | Claude (or OpenRouter via `ANTHROPIC_BASE_URL`, "may not work correctly" with non-Anthropic models [V-docs]) | Claude Code's own harness | Options include `canUseTool`, `hooks` (PreToolUse, SubagentStart/Stop, WorktreeCreate, …), `agents`, `outputFormat`, `resume`/`sessionId`/`forkSession`, `includePartialMessages`, `maxBudgetUsd`, `sessionStore`, `pathToClaudeCodeExecutable` [V-src `sdk.d.ts`] | Proprietary licence ("© Anthropic PBC. All rights reserved"); optionalDependencies ship native per-platform binaries [V-src]; **API key only for third-party products** [V-docs] | Skip as a separate backend. Use the **Claude CLI adapter with `ANTHROPIC_API_KEY` env** for BYOK. |
| **OpenAI Agents SDK** (JS 0.21.0 / Py 0.24.0) | OpenAI; others via `setDefaultOpenAIClient(new OpenAI({baseURL}))` + `setOpenAIAPI('chat_completions')` or the AI-SDK adapter in `@openai/agents-extensions` [V-src] | Primitives (agents, handoffs, guardrails, tracing, `SandboxAgent`), **no coding harness out of the box** | Stream events `raw_model_stream_event`, `run_item_stream_event` (`message_output_created`, `tool_called`, `tool_output`, `handoff_occurred`, `reasoning_item_created`, `tool_approval_requested`), `agent_updated_stream_event`; HITL through `needsApproval` plus a serializable `RunState` (`toJSON`/`fromString`) [V-src] | MIT | Good for a custom harness later. Today it means more to build than Deep Agents. |
| **Codex SDK** (0.164.0) | Codex models (ChatGPT login or API key) | Codex harness | Wraps the `codex` CLI over JSONL (`thread.started`, `item.*`, `turn.completed{usage}`); `resumeThread(id)`, `outputSchema` [V-src README] | Apache-2.0 (CLI) [I] | This *is* the Codex adapter path, not a separate backend. |
| **BYOK into vendor CLIs** | Claude Code: `ANTHROPIC_API_KEY`, or OpenRouter via `ANTHROPIC_BASE_URL=https://openrouter.ai/api`, `ANTHROPIC_AUTH_TOKEN=$OPENROUTER_API_KEY`, `ANTHROPIC_API_KEY=""` [V-docs]. Codex: `OPENAI_API_KEY` or a `[model_providers.x]` with `base_url`/`env_key`/`wire_api="responses"` (chat removed) [V-src] | Best-in-class per vendor | Same adapters, same events | Nothing new to ship | **Do this first for Anthropic and OpenAI keys.** |

---

## 4. (B1) Orchestration patterns that work for coding agents in 2026

### 4.1 Evidence base

| Source | Key finding | Tag |
|---|---|---|
| Anthropic, *How we built our multi-agent research system* (2025-06-13) | Opus-4 lead with Sonnet-4 subagents beat single-agent Opus 4 by **90.2%** on an internal research eval. Token usage explains **80%** of BrowseComp variance. Agents use about **4x** the tokens of chat, multi-agent about **15x**. Parallelism cut research time by up to 90%. *"Most coding tasks involve fewer truly parallelizable tasks than research"*; LLMs are *"not yet great at coordinating and delegating to other agents in real time."* Delegation needs an **objective, an output format, tool guidance and boundaries**. Subagents write to external storage and pass references (avoid the "game of telephone"). Make runs resumable from errors, with checkpoints plus retries. | [V-docs] (numbers are [C]) |
| Google Research, *Towards a science of scaling agent systems* (blog 2026-01-28; arXiv 2512.08296 v3 2026-04-08) | 180 configurations. Centralized coordination gave **+80.9%** on Finance-Agent (parallelizable). On PlanCraft (sequential), **every multi-agent variant lost 39–70%**. Error amplification was **17.2x** for independent agents vs **4.4x** centralized. Tool-heavy tasks (16+ tools) carry disproportionate coordination cost. There is a capability-saturation effect: no exact threshold is stated in the abstract, and the "~45%" figure seen in commentary is unverified. | [V-docs] / [C] |
| Cognition, *Multi-Agents: What's Actually Working* (Walden Yan, 2026-04-22), a follow-up to *Don't Build Multi-Agents* (2025-06) | What works: a **generator-verifier loop** where the reviewer starts with a **clean context** and sees only the diff (Devin Review averages about 2 bugs per PR, ~58% severe [C]); "smart friend" escalation across frontier models; manager and child Devins (still hard); read-only subagents. What doesn't: **parallel writers**, because actions carry implicit decisions. **Writes should stay single-threaded.** The preferred shape is "map-reduce-and-manage". | [V-docs] |
| Claude Code **agent teams** docs (experimental, `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`) | A lead and teammates share a task list (pending, in progress, completed, with dependencies). *"Task claiming uses file locking to prevent race conditions."* Mailboxes live at `~/.claude/teams/{team}/inboxes/{agent}.json`; tasks at `~/.claude/tasks/{team}/`. Hooks `TeammateIdle`, `TaskCreated` and `TaskCompleted` (exit 2 blocks and sends feedback). "Start with **3–5 teammates**", "**5–6 tasks per teammate**", *"Two teammates editing the same file leads to overwrites."* **There are no teammates in `-p`/SDK mode.** | [V-docs] |
| Claude Code **subagents** docs | Frontmatter `isolation: worktree` (a temporary worktree branched from the default branch, auto-cleaned if unchanged), `background`, `maxTurns`, `permissionMode`, `model`, `effort`, `skills`, `mcpServers`, `hooks`, `memory`. Nesting is up to 3 levels (`CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`). Resume works through `SendMessage` to the agent id. `--agents '<json>'` (or a file path with `-p`, ≥2.1.281). | [V-docs] |
| Anthropic, *A harness for every task: dynamic workflows in Claude Code* (2026-06-02) | Scripted orchestration with `agent(prompt,{schema, model, isolation:"worktree"})`, `parallel()` and `pipeline()`. Patterns: classify-and-act, fan-out-and-synthesize (with a barrier), **adversarial verification**, generate-and-filter, tournament, loop-until-done. Single-context failure modes: "agentic laziness", "self-preferential bias", "goal drift". Example run: review-changes used 14 agents, 482k tokens, 6 min. *"Parallelism and specialization have to earn their coordination cost."* *"Most traditional coding tasks do not need a panel of 5 reviewers."* | [V-docs] |
| OpenAI Codex best practices (now at learn.chatgpt.com/guides/best-practices) | Prompts need Goal, Context, Constraints and **Done when**. Plan first for complex work. Short, accurate AGENTS.md. Have Codex write and run tests, then `/review`. Use subagents for bounded work (exploration, tests, triage). Listed common mistake: *"Running live tasks on the same files without using Git worktrees."* *"Fork only when the work truly branches."* | [V-docs] |
| Xu, Subramanian, Karthik, *AI Agent Pull Requests on GitHub: Frequency, Structure, and Merge Conflict Rates* (arXiv 2607.04697, 2026-07) | 33,596 agent PRs in 2,807 repos. Co-active PR pairs exist in 40.2% of repos. Replayed 3-way merges conflict in **19.8%** of same-agent pairs and **41.7%** of cross-agent pairs. 84.4% of conflicted files are source code. About 42% are structural (modify/delete, add/add). | [V-docs] |
| Cemri et al., *Why Do Multi-Agent LLM Systems Fail?* (MAST taxonomy, 2025) | Failures cluster into spec/design issues, inter-agent misalignment, and weak verification or termination. | [I: from memory, not re-fetched] |

### 4.2 Pattern: orchestrator–worker over a task DAG (adopt)

- **One** planner produces the DAG, with acceptance criteria, write scopes and verify commands per task. A **deterministic** scheduler executes it.
- Don't let LLM workers plan the global schedule. The 17.2x vs 4.4x error amplification argues for a central validator.
- The delegation contract follows Anthropic's four fields: objective, output format, tool guidance, boundaries. That means: goal, a JSON report schema, allowed commands, and write scope plus "don't touch".
- Scale effort with complexity, following Anthropic's heuristic:
  - Simple work: 1 agent.
  - Moderate work: 2–4 workers.
  - More than about 5 parallel writers is rarely justified for coding.

### 4.3 Parallel workers in isolated worktrees (adopt, with limits)

- Use one worktree and one branch per task: `git worktree add -b codly/<plan>/<task> <path> <integration_head>`. Worktrees remove *filesystem* interference, but not *semantic* conflicts: the 19.8–41.7% conflict rates above happened despite separate branches.
- Base dependents on the integration branch **after** their dependencies merge, not on the original base.
- macOS tip: seed per-worktree build caches with APFS clones (`cp -c`, i.e. clonefile(2) [V-local `man cp`]) for `node_modules` and `target/`, instead of reinstalling. [I]

### 4.4 File ownership and locking (adopt)

- Every task declares `write_scope` globs. While a task runs, the orchestrator holds **scope leases**, and a ready task whose scope intersects a leased scope waits. This is Claude teams' "each teammate owns a different set of files", made mechanical.
- **Hot files** go to a single task that others depend on: lockfiles, `package.json`, `Cargo.toml`, route registries, barrel `index.ts`, DB migrations, i18n catalogs. The planner prompt and a validator rule both enforce this.
- After the run, check the diff against scope (§6.7). Leases are advisory for the agent but enforced at the gate.

### 4.5 Shared board vs messaging (blackboard wins for v1)

- **Blackboard** (Codly-owned SQLite task board) is the **single source of truth**. Agents don't message each other directly in v1. Cognition reports that child-to-sibling discovery propagation is still an open problem, and Claude teams' own limitations list "task status can lag".
- **Agent → orchestrator signals** go through a **Codly MCP server** exposed to every CLI. Claude, Codex, Gemini, OpenCode and Cursor all support MCP [I: per sibling reports]. It offers `codly_report_blocker(reason)`, `codly_post_note(text, affects:[task_keys])` and `codly_get_task()`. The orchestrator decides whether a note is injected into dependent tasks' prompts at *their* start, or forwarded to a running worker as a follow-up turn.
- Persist "discoveries" as `message` rows. The Living World can visualize them as speech bubbles, with real data only.

### 4.6 Verification and test-gated completion (adopt, most important)

Run the gates in this order, cheapest first:

1. **Deterministic gate (Codly runs it, not the agent).** Each task's `verify` commands plus repo defaults (typecheck, lint, unit tests for touched packages), with a timeout, inside the worktree. A non-zero exit sends feedback to the same worker session (resume) with the failing output tail.
2. **Scope gate.** `git diff --name-only base..HEAD` ⊆ `write_scope`. Otherwise mark it a scope violation and send it to the reviewer.
3. **Reviewer agent.** A **clean context** with only the task goal, the acceptance criteria and the diff (Cognition). Use a **different vendor or model** from the worker where available, to counter self-preferential bias. It returns structured JSON (§6.7). `request_changes` goes back to the worker; `reject` sends the task back to planning or to a human.
4. **Integration gate** after merging into the integration branch: run the plan-level `integration_checks`.
5. **Human gate:** the plan result, before the user merges. Earlier per policy for `risk: high` tasks.

### 4.7 Retries and recovery

| Failure | Detection | Action |
|---|---|---|
| Transient (429, "overloaded", network) | adapter error classes, exit codes, `api_retry` events | Exponential backoff with jitter. Resume the **same** session. Doesn't count against attempts. |
| Plan-quota exhaustion | adapter rate-limit signals (Claude SDK types include `five_hour`/`seven_day` windows [V-src]; Codex `account/rateLimits/*` per sibling report) | Pause that vendor's queue. Optionally reassign to another vendor if the user allows. |
| Verify failure | non-zero gate | Feedback to the same session. Default max 2 fix loops. |
| Stalled agent | no events for N min, or turn or budget limit hit | Cancel, then resume with "summarize progress, continue". Second stall: reassign. |
| Review `request_changes` | reviewer JSON | Same session with the issues list. Max 2. |
| Merge conflict | `git merge` non-zero | `--abort`, then the worker rebases onto integration HEAD in its worktree and re-verifies. |
| App crash | startup reconciliation (§6.9) | Kill orphans and resume by session id. |

Every step persists its state transition **before** the side effect, and side effects are idempotent (§6.9).

### 4.8 When NOT to parallelize (rules for the planner and scheduler)

Default to **one worker** when any of these holds:

1. The tasks form a chain (each needs the previous one's output). Google: −39–70% on sequential work.
2. Write scopes overlap or touch hot files.
3. Total work is small (S-sized, or under about 3 files). Coordination overhead dominates.
4. The spec is ambiguous. Plan or clarify first (Codex best practice).
5. It is debugging a single failure. Run hypotheses in parallel as **read-only** investigations if desired, but keep the fix single-threaded.
6. Several cross-vendor agents would edit the same module. Cross-agent conflicts are about 2x more likely than same-agent ones.
7. The user is on a subscription close to its limits. About 15x token multipliers are real.

---

## 5. (B2) Cheap routing and decision layer

### 5.1 Decisions Codly must make

| Decision | When | Default method |
|---|---|---|
| Which agent or model for a task | once per task at schedule time | rules, then cheap model |
| Is more context needed before starting | once per task | rules (missing files or criteria), then cheap model |
| Parallelize or not | at plan validation | **deterministic** (§4.8) |
| Does this tool call need human approval | per tool call | **deterministic policy**; cheap model only for unknown shell commands |
| Is the task done | at the gate | **deterministic tests**, then the reviewer |
| Retry, reassign or escalate | on failure | rules |

### 5.2 Three tiers [I]

- **L0, rules (free, about 0 ms).** User pins, agent availability and quota, a file-type and size heuristic, a command allowlist and denylist, path policies (outside the worktree, `.env`, `~/.ssh`, `.git/`), and network or destructive patterns (`rm -rf`, `git push`, `git reset --hard`, `curl | sh`, `sudo`, `chmod -R`). About 80% of decisions should end here.
- **L1, cheap structured model (about 0.3–4 s).** One call with a **multi-question schema**: batch the decisions about a task into one call, the same trick as Jev's parallel questions. Backends, in preference order:
  1. A Jev or OpenRouter cheap model in API-key mode.
  2. `claude -p --model haiku` or `codex exec -m gpt-6-luna` in subscription mode.
  3. Optionally, a local model.
- **L2, escalate.** A stronger model, or the **human** when L1 confidence is below the per-decision threshold or the risk class is high.

### 5.3 Using CLI subscriptions for cheap calls (verified today) [V-local]

**Claude Code 2.1.309, Haiku:**

```bash
claude -p --model haiku --effort low --output-format json --no-session-persistence \
  --tools "" --setting-sources "" --strict-mcp-config \
  --system-prompt "You are a routing classifier for a coding-agent orchestrator. Answer only via the structured output." \
  --json-schema '{"type":"object","properties":{"agent":{"type":"string","enum":["claude","codex","gemini"]},"needs_more_context":{"type":"boolean"},"confidence":{"type":"number","minimum":0,"maximum":1}},"required":["agent","needs_more_context","confidence"],"additionalProperties":false}' \
  "Task: 'Rename the CSS class .btn-primary to .btn-main across the web/ folder'." < /dev/null
```

The result (abridged, real):

```json
{"type":"result","subtype":"success","is_error":false,"num_turns":2,
 "result":"{\"agent\":\"claude\",\"needs_more_context\":false,\"confidence\":0.85}",
 "structured_output":{"agent":"claude","needs_more_context":false,"confidence":0.85},
 "session_id":"b859c9f5-…","total_cost_usd":0.0004735,"stop_reason":"tool_use","terminal_reason":"completed",
 "usage":{"input_tokens":2,"cache_creation_input_tokens":1539,"output_tokens":331,…},
 "modelUsage":{"claude-haiku-5-5":{…,"costBasis":"list"}}}
```

What the probe showed:

- About **3.2 s wall** and about 2.0 s `duration_ms`.
- `haiku` resolves to `claude-haiku-5-5`.
- Without `--system-prompt`, the default prompt cost 4,591 cache-write tokens and took 4.3 s.
- `total_cost_usd` is a **list-price figure** (`costBasis:"list"`). On a subscription, the call consumes plan quota instead.
- `num_turns:2`: structured output is implemented as an internal tool turn.
- `--no-session-persistence` still returns a `session_id`.

**Codex CLI 0.161.0, ChatGPT login:**

```bash
codex exec --json --ephemeral --ignore-user-config --skip-git-repo-check -s read-only \
  -m gpt-6-luna -c model_reasoning_effort='"low"' --output-schema schema.json \
  "Route this task to one agent: '…'. Do not run any commands." < /dev/null
```

The output is JSONL (real):

```json
{"type":"thread.started","thread_id":"01a17505-69fe-7a12-8591-21db337ea43f"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"{\"agent\":\"codex\",\"needs_more_context\":false,\"confidence\":0.99}"}}
{"type":"turn.completed","usage":{"input_tokens":15074,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":28,"reasoning_output_tokens":0}}
```

What the probe showed:

- About **3.7 s wall**. The ~15k input tokens are fixed harness overhead.
- **Gotchas:**
  1. Without `< /dev/null`, Codex prints "Reading additional input from stdin..." and **blocks forever** when stdin is a pipe. Adapters must use `Stdio::null()`.
  2. `-m gpt-5.4-mini` failed with `"The 'gpt-5.4-mini' model is not supported when using Codex with a ChatGPT account."`. Read the allowed slugs from `~/.codex/models_cache.json`. Today it lists `gpt-6.1-sol`, `gpt-6-sol`, `gpt-6-astra`, `gpt-6-luna`, `gpt-5.6-sol/terra/luna` (plus hidden `codex-auto-review`).
  3. `--ignore-user-config` avoids loading the user's MCP servers (one of them errored on startup) and keeps the call cheap. Auth still comes from `CODEX_HOME`.
- The parsed JSON is the `agent_message.text`. Validate it against the schema in Rust (the `jsonschema` crate).

**Gemini CLI 0.62.0** has `-p`, `-o json|stream-json`, `--approval-mode plan` and `--session-id`, but **no JSON-schema flag** [V-local help]. Ask for JSON and validate it, or skip Gemini for structured decisions.

**Implications [I]:**

- Subscription decision calls cost about 3–4 s and real quota. Use them **per task** (routing plus a context check in one call) and **never per tool call**.
- For approvals on a subscription, L0 rules must cover nearly everything. Anything else goes to a human.
- Prefer Claude's `--json-schema` (native `structured_output`) when both are available.

### 5.4 "TypeSafe Jev": what it is

It exists, it is recent, and it fits the problem of cheap calibrated decisions:

- **Product:** TypeSafe AI's "System One" model **Jev**. You send `state` (text or JSON) plus typed `questions` and get structured answers, not prose. [V-docs docs.typesafe.ai]
  - **Choice:** `choice`, `probabilities`, `confidence`.
  - **Score:** a rubric with `score`, `legend`, `probabilities`, `confidence`.
  - **Noul:** yes/no as a probability `noul` ∈ [0,1].
  - Questions in one request run **in parallel**. A vendor cookbook claims 13 questions in one call are 11.5x cheaper and 9.6x faster than 13 calls [C].
- **API:** `POST https://api.typesafe.ai/v1/systemone`, `Authorization: Bearer <key>`, body `{state, model:"jev-latest", questions:{<id>:{type, instructions, criteria?}}}`. The example response shows `"model":"jev-1.13.0"` and `usage{input_tokens,output_tokens}`. SDKs: `@typesafe-ai/sdk` 0.7.0 (`systemOne`) and `typesafe-sdk` 0.7.5 (`TypeSafeClient.system_one`), env `TYPESAFE_API_KEY`. [V-docs][V-reg]
- **Distribution and price:** Vercel AI Gateway `typesafe-ai/jev` at **$0.042 / 1M input tokens**, 32k context, providers typesafe-ai and DigitalOcean [C, Vercel page]. OpenRouter lists `typesafe/jev-router` (chat-completions interface, pricing reported as `-1` in the models API) [V-local OpenRouter `/api/v1/models`].
- **Confidence guidance from the vendor** [V-docs `confidence.md`]:
  - Thresholds should scale with risk.
  - The 0.5 escalation floor and 0.9 auto-act values are "illustrations, not recommendations".
  - Noul starting points are 0.7 for yes and 0.3 for no.
  - **No ECE or Brier numbers are published there.**
- **Independent signals (community; [C]):**
  - KoBBQ audit: Jev chose "unknown" on 95% of ambiguous items.
  - A CLINC150 conformal "jev-certify" study: a 5% silent-misroute bound held in-scope, but the scope gate missed 3.6x when out-of-scope prevalence rose.
  - A 111-case agent-action-gate study: Jev matched 100 labels vs Claude 102, with one unsafe allow each.
  - JevBench.
  - Projects: `BorisLeMeec/jev` (a Claude Code plugin using Jev for codebase questions), `dzhng/jevgrep` / `nassim-arifette/jevgrep` (semantic code search; "~30% lower cost" claim [C]), `rmax-ai/jevgrep-eval`.
- **For Codly [I]:** an **optional L1 backend in API-key mode** (TypeSafe key, or through OpenRouter or Vercel). It suits routing, "needs more context?" and Noul-style gray-zone approval checks ("Does this command modify files outside the worktree or access the network?"). It is **not** a planner. The vendor says it is weak at multi-step reasoning and interdependent questions. Do not use its probabilities as calibrated until Codly's own outcome log confirms it (§5.6).

### 5.5 Local model option [I]

A local 3–8B model (Ollama, LM Studio or MLX) gives real **token logprobs**: constrain the output to label tokens and read the probability mass per label. That beats verbalized confidence for calibration. The costs:

- A 2–5 GB download.
- RAM pressure next to several agent CLIs.
- Model management UX.

Ship it as an opt-in "offline router", not the default. Codex also supports `--oss --local-provider ollama|lmstudio` [V-local help].

### 5.6 Calibration: what to trust and how to calibrate in Codly

**Signals available per backend:**

| Backend | Signal |
|---|---|
| CLI subscriptions (Claude, Codex) | Verbalized confidence only. No logprobs. |
| Anthropic API | No token logprobs to my knowledge [I: not re-verified today]. |
| OpenAI and OpenRouter APIs | `logprobs` for some non-reasoning models/providers [I]. |
| Jev | Native probability vectors [V-docs]. |
| Local | Logprobs. |

**What the literature says:**

- *Just Ask for Calibration* (Tian et al., arXiv 2305.14975): for RLHF models, **verbalized** probabilities are often *better* calibrated than token probabilities. Numbers work at least as well as words. CoT did not help. [V-docs search summary]
- *Can LLMs Express Their Uncertainty?* (Xiong et al., arXiv 2306.13063, ICLR 2024): verbalized confidence is **highly overconfident**. **Consistency-based** methods (sample k answers and measure agreement) beat verbalized confidence in most cases, and **hybrids are best**. [V-docs search summary]
- **Conformal and selective prediction** (from memory, IDs not re-fetched [I]): Angelopoulos and Bates, *Gentle Introduction to Conformal Prediction* (2107.07511); *Learn Then Test* (2110.01052); *Conformal Risk Control* (2208.02814). These give distribution-free guarantees such as "error rate among auto-accepted ≤ α", under exchangeability.
- **Cascades and routers:** FrugalGPT (2305.05176) and RouteLLM (2406.18665) [I: from memory] show that cheap-first cascades with a learned deferral threshold preserve quality at a fraction of the cost.

**Codly procedure [I]:**

1. **Log every decision** in `decision` (§6.2) with features, predicted label, raw score, method and model. Record the **outcome later**:
   - Routing: task passed verify and review on the first attempt, plus no user override.
   - Context: the agent did or did not report a blocker for missing info.
   - Approval gray zone: the user's actual approve or deny.
2. **Before about 200 labeled outcomes per decision type**, use fixed conservative policies:
   - Low-risk decisions auto at score ≥ 0.6, otherwise default routing.
   - Medium-risk decisions auto at ≥ 0.85, otherwise ask the user.
   - High-risk decisions are **always human**.
3. **After that, recalibrate:** isotonic regression (Platt scaling if n < 500) from raw score to P(correct), per decision type and backend. Show the reliability curve in a debug panel.
4. **Selective threshold with a guarantee:** choose τ as the smallest threshold whose **upper 95% Clopper–Pearson bound** on the error rate among calibration items with score ≥ τ is ≤ α. Use α = 5% for routing and 1% for approval auto-allow. Recompute nightly, and only from **exchangeable** data: same backend, model and version, since a CLI upgrade resets calibration.
5. **Optional self-consistency for high-value decisions:** k = 3 samples at temperature, and use the agreement rate as the score. That costs 3x, so use it only for planner-level choices.
6. **Risk-tiered gating beats a single global threshold.** TypeSafe and Claude docs say the same.

### 5.7 Decision schemas (one call per task)

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["agent", "p_success", "needs_more_context", "missing_context", "risk", "rationale"],
  "properties": {
    "agent": {"type": "string", "enum": ["claude", "codex", "gemini", "opencode", "deepagents"]},
    "p_success": {"type": "object", "additionalProperties": false,
      "required": ["claude", "codex", "gemini", "opencode", "deepagents"],
      "properties": {"claude": {"type": "number"}, "codex": {"type": "number"}, "gemini": {"type": "number"},
                     "opencode": {"type": "number"}, "deepagents": {"type": "number"}}},
    "needs_more_context": {"type": "boolean"},
    "missing_context": {"type": "array", "items": {"type": "string"}},
    "risk": {"type": "string", "enum": ["low", "medium", "high"]},
    "rationale": {"type": "string"}
  }
}
```

Asking for a **probability per option** (`p_success`) instead of a single confidence gives a distribution. Codly can normalize it, compute the top-1 vs top-2 margin, and recalibrate. Keep `rationale` under about 200 characters and show it in the explain panel, labeled "model's stated rationale". All fields are `required` with `additionalProperties:false`, so the same file works for `--json-schema` and the stricter `--output-schema`.

---

## 6. (B3) Orchestrator design for Codly's vertical slice

### 6.1 Flow

```
User request
  → [L0/L1 router] pick planner backend (default: strongest available model)
  → PLAN: 1 planner run (read-only tools) → JSON DAG → Codly validates/normalizes → user approves (or auto if S-sized)
  → SCHEDULE: ready tasks (deps merged) × scope leases × max_parallel × per-vendor caps
  → EXECUTE: per task: create worktree from integration HEAD → worker run via adapter → JSON report
  → GATE: verify cmds (Codly) → scope check → reviewer run (other vendor, clean context) → [fix loop ≤2]
  → MERGE QUEUE: serial merge into codly/<plan>/integration → integration checks → release leases → unblock dependents
  → DONE: present integration branch + summary + cost; user merges/opens PR (Codly never pushes by default)
```

### 6.2 Data model (SQLite, WAL) [I]

```sql
PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;

CREATE TABLE workspace (
  id TEXT PRIMARY KEY, repo_path TEXT NOT NULL UNIQUE, default_branch TEXT NOT NULL,
  created_at INTEGER NOT NULL);

CREATE TABLE plan (
  id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspace(id),
  request TEXT NOT NULL,                              -- the user's words, verbatim
  status TEXT NOT NULL CHECK (status IN ('planning','awaiting_approval','running','integrating','done','failed','cancelled')),
  base_commit TEXT NOT NULL,
  integration_branch TEXT NOT NULL,                   -- codly/<plan>/integration
  integration_worktree TEXT,
  planner_run_id TEXT,
  plan_json TEXT NOT NULL DEFAULT '{}',               -- validated + normalized planner output (immutable after approval)
  max_parallel INTEGER NOT NULL DEFAULT 3,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);

CREATE TABLE agent (                                  -- a "character" in the Living World
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,                                 -- claude|codex|gemini|opencode|cursor|copilot|pi|deepagents
  role TEXT NOT NULL,                                 -- planner|worker|reviewer|router|merger
  display_name TEXT NOT NULL, model TEXT, sprite TEXT,
  status TEXT NOT NULL DEFAULT 'idle',                -- projection of latest events (see §6.11)
  current_run_id TEXT);

CREATE TABLE task (
  id TEXT PRIMARY KEY, plan_id TEXT NOT NULL REFERENCES plan(id),
  key TEXT NOT NULL,                                  -- 't1', stable within plan
  title TEXT NOT NULL, goal TEXT NOT NULL,
  acceptance_json TEXT NOT NULL,                      -- ["criterion", ...]
  write_scope_json TEXT NOT NULL, read_scope_json TEXT NOT NULL,   -- repo-relative globs
  verify_json TEXT NOT NULL,                          -- ["pnpm -F web test", ...]
  size TEXT CHECK (size IN ('S','M','L')), risk TEXT NOT NULL DEFAULT 'medium',
  agent_hint TEXT,
  status TEXT NOT NULL CHECK (status IN ('pending','ready','running','verifying','reviewing',
           'changes_requested','approved','merging','done','failed','blocked','cancelled')),
  assigned_agent_id TEXT REFERENCES agent(id),
  branch TEXT, worktree_path TEXT, base_commit TEXT, head_commit TEXT,
  attempt INTEGER NOT NULL DEFAULT 0, max_attempts INTEGER NOT NULL DEFAULT 3,
  critical_path INTEGER NOT NULL DEFAULT 0,           -- longest path to a sink (scheduling priority)
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  UNIQUE (plan_id, key));

CREATE TABLE task_dep (
  task_id TEXT NOT NULL REFERENCES task(id), depends_on TEXT NOT NULL REFERENCES task(id),
  PRIMARY KEY (task_id, depends_on));

CREATE TABLE scope_lease (                            -- held from 'running' until 'done'/'failed'
  task_id TEXT NOT NULL REFERENCES task(id), glob TEXT NOT NULL,
  acquired_at INTEGER NOT NULL, released_at INTEGER,
  PRIMARY KEY (task_id, glob));

CREATE TABLE run (                                    -- one OS process lifetime (or sidecar run)
  id TEXT PRIMARY KEY,
  plan_id TEXT REFERENCES plan(id), task_id TEXT REFERENCES task(id),
  agent_id TEXT NOT NULL REFERENCES agent(id),
  role TEXT NOT NULL, adapter TEXT NOT NULL, adapter_version TEXT, model TEXT,
  cwd TEXT NOT NULL, argv_json TEXT NOT NULL,         -- secrets redacted
  session_id TEXT,                                    -- provider session/thread id; PRE-ASSIGNED when possible
  resumed_from_run_id TEXT REFERENCES run(id),
  pid INTEGER, pgid INTEGER, proc_start_time INTEGER, -- to detect PID reuse
  status TEXT NOT NULL CHECK (status IN ('starting','running','waiting_approval','completed','failed','cancelled','lost')),
  exit_code INTEGER, error TEXT,
  input_tokens INTEGER NOT NULL DEFAULT 0, output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL, cost_source TEXT CHECK (cost_source IN ('provider','estimated','list_price','unknown')),
  raw_log_path TEXT NOT NULL,                         -- runs/<id>/raw.ndjson, written BEFORE parsing
  last_seq INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER, ended_at INTEGER);

CREATE TABLE event (                                  -- append-only; both views are projections of this
  run_id TEXT NOT NULL REFERENCES run(id), seq INTEGER NOT NULL, ts INTEGER NOT NULL,
  ns TEXT NOT NULL DEFAULT '',                        -- subagent path joined by '/'
  type TEXT NOT NULL,                                 -- AgentEvent tag
  payload TEXT NOT NULL,                              -- JSON
  PRIMARY KEY (run_id, seq));
CREATE INDEX event_by_type ON event(type, ts);

CREATE TABLE message (                                -- blackboard notes + user↔agent turns
  id TEXT PRIMARY KEY, plan_id TEXT REFERENCES plan(id), task_id TEXT REFERENCES task(id),
  from_kind TEXT NOT NULL CHECK (from_kind IN ('user','agent','orchestrator')), from_id TEXT,
  to_kind TEXT NOT NULL CHECK (to_kind IN ('agent','task','plan','user')), to_id TEXT,
  kind TEXT NOT NULL,                                 -- instruction|question|answer|note|blocker|review_feedback
  body TEXT NOT NULL, delivered_run_id TEXT REFERENCES run(id), created_at INTEGER NOT NULL);

CREATE TABLE artifact (
  id TEXT PRIMARY KEY, task_id TEXT REFERENCES task(id), run_id TEXT REFERENCES run(id),
  kind TEXT NOT NULL,                                 -- plan|diff|patch|test_report|review|worker_report|log|file
  path TEXT, sha256 TEXT, summary TEXT, created_at INTEGER NOT NULL);

CREATE TABLE approval (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES run(id), task_id TEXT REFERENCES task(id),
  adapter_request_id TEXT NOT NULL,                   -- control_request id / ACP request id / interrupt_id
  tool_name TEXT NOT NULL, args_json TEXT NOT NULL,
  risk TEXT NOT NULL CHECK (risk IN ('low','medium','high')),
  policy TEXT NOT NULL CHECK (policy IN ('auto_allow','auto_deny','ask')),
  decision_id TEXT REFERENCES decision(id),           -- why the policy said what it said
  status TEXT NOT NULL CHECK (status IN ('pending','approved','rejected','edited','expired')),
  decided_by TEXT, decided_at INTEGER, reason TEXT, created_at INTEGER NOT NULL);

CREATE TABLE decision (                               -- explainability + calibration log
  id TEXT PRIMARY KEY, plan_id TEXT, task_id TEXT, run_id TEXT,
  type TEXT NOT NULL,      -- route|context|plan_validate|schedule|gate|review|merge|retry|approval_policy
  method TEXT NOT NULL,    -- rule|model|human
  backend TEXT, model TEXT, model_version TEXT,
  inputs_json TEXT NOT NULL,          -- features actually used
  output_json TEXT NOT NULL,          -- chosen option (+ alternatives with scores)
  raw_score REAL, calibrated_score REAL, threshold REAL,
  rationale TEXT,                     -- rule text or model's stated rationale (labeled as such)
  outcome TEXT, outcome_at INTEGER,   -- filled later for calibration (§5.6)
  cost_usd REAL, latency_ms INTEGER, created_at INTEGER NOT NULL);
```

Rust side: `rusqlite 0.41` (bundled) with a single writer task (an mpsc channel) and read connections for queries. All IDs are UUIDv7 or ULID, so they sort by time.

### 6.3 Task state machine

```
pending ──deps merged──▶ ready ──lease+slot──▶ running ──report──▶ verifying ──pass──▶ reviewing ──approve──▶ approved
   ▲                       │                     │  ▲                  │fail                │request_changes      │
   │                       │                     │  └──(attempt<max)───┴────────────────────┘                     ▼
   │                    blocked ◀── blocker ─────┘                                                             merging
   │                                                                                                          │   │conflict
 (re-plan) ◀── failed ◀── attempts exhausted / reject                                                    done ◀┘   └──▶ changes_requested
```

Every transition is written in one SQLite transaction together with its `decision` row. The UI animates transitions from the event log, never from guesses.

### 6.4 Planner step

**One call, read-only tools, structured output.**

- **Claude [V-local flags; combination untested]:** `claude -p --model opus --effort high --output-format json --json-schema "$(cat plan.schema.json)" --tools "Read,Grep,Glob" --session-id <uuid> "<prompt>"` with cwd = repo root.
- **Codex [V-local flags]:** `codex exec --json -C <repo> -s read-only --output-schema plan.schema.json "<prompt>" < /dev/null`.
- **Deep Agents:** `responseFormat: zodPlan` with a `FilesystemBackend` read-only `permissions` set.

**Prompt skeleton:**

```
You are the planner for Codly. Produce a task DAG for the request below.
Rules:
- Prefer FEWER tasks. A single task is correct for small or tightly coupled changes.
- Split only along independent file areas. Each task lists write_scope globs it alone may modify.
- Put dependency/lockfile/manifest/migration/route-registry edits in ONE task others depend on.
- Every task: 1–5 testable acceptance criteria + shell `verify` commands that exit 0 when done.
- depends_on lists task ids whose MERGED output this task needs.
- Max 8 tasks. Do not write code.
Repo context: <tree summary, AGENTS.md/CLAUDE.md excerpts, package scripts, git status, recent commits>
Request: <user words>
```

**`plan.schema.json`** (strict-compatible: all fields required, `additionalProperties:false`; Codly enforces counts and patterns itself):

```json
{
  "type": "object", "additionalProperties": false,
  "required": ["summary", "assumptions", "open_questions", "integration_checks", "tasks"],
  "properties": {
    "summary": {"type": "string"},
    "assumptions": {"type": "array", "items": {"type": "string"}},
    "open_questions": {"type": "array", "items": {"type": "string"}},
    "integration_checks": {"type": "array", "items": {"type": "string"}},
    "tasks": {"type": "array", "items": {
      "type": "object", "additionalProperties": false,
      "required": ["id", "title", "goal", "acceptance_criteria", "write_scope", "read_scope",
                   "depends_on", "verify", "size", "risk", "agent_hint"],
      "properties": {
        "id": {"type": "string"}, "title": {"type": "string"}, "goal": {"type": "string"},
        "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
        "write_scope": {"type": "array", "items": {"type": "string"}},
        "read_scope": {"type": "array", "items": {"type": "string"}},
        "depends_on": {"type": "array", "items": {"type": "string"}},
        "verify": {"type": "array", "items": {"type": "string"}},
        "size": {"type": "string", "enum": ["S", "M", "L"]},
        "risk": {"type": "string", "enum": ["low", "medium", "high"]},
        "agent_hint": {"type": "string", "enum": ["any", "claude", "codex", "gemini", "opencode", "deepagents"]}
      }}}
  }
}
```

**Deterministic validation and normalization (Rust) [I].** Every fix is logged as a `decision(type='plan_validate')`.

1. Check the JSON Schema with `jsonschema 0.60`. Enforce 1–8 tasks, unique `id`s, at least one criterion and at least one `verify` per task (or inherit the repo default verify).
2. Check the dependencies form a DAG: toposort with `petgraph 0.8`. On a cycle, reject and re-plan once with the error message, then fall back to a single task.
3. Check scope overlap for every pair of tasks **not ordered** by the DAG. If their write scopes intersect, **add an edge** (the later id depends on the earlier one) or merge them if both are S-sized.
4. Hot-file rule: any task touching a hot file becomes a dependency of every other task that writes in the same package.
5. "Not worth parallelizing" collapse: if every task is S-sized and the DAG has width ≤ 1 (or total writes are ≤ 3 files), merge into one task.
6. Compute `critical_path` for priority.

**Glob intersection [I]:** use `globset 0.4.20`.

- Concrete: enumerate `git ls-files` and check whether any path matches both sets.
- Speculative, for new files: compare literal prefixes up to the first wildcard. If one is a prefix of the other, treat the scopes as overlapping.

This is conservative by design.

### 6.5 Scheduler

```rust
// Runs on every task/run state change and every 2 s tick. Pure function of DB state → actions.
fn schedule(db: &Db, cfg: &Cfg) -> Vec<Action> {
    let running = db.running_tasks();
    let mut free = cfg.max_parallel.saturating_sub(running.len());       // default 3
    let mut leased: GlobSet = db.active_leases();
    let mut per_vendor = db.running_count_by_vendor();                   // caps: claude 2, codex 2, gemini 2, deepagents 3
    let mut actions = vec![];
    for t in db.ready_tasks_ordered_by(|t| (Reverse(t.critical_path), t.created_at)) {
        if free == 0 { break; }
        if !db.deps_merged(&t) { continue; }                             // gate on MERGED, not "done"
        if scopes_overlap(&t.write_scope, &leased) {                     // file-ownership lease
            actions.push(Action::Explain(t.id, "waiting: write scope overlaps running task"));
            continue;
        }
        let route = router.route(&t);                                    // L0 rules → L1 model (§5); logs a decision
        if per_vendor[route.vendor] >= cfg.cap(route.vendor) || quota.exhausted(route.vendor) {
            actions.push(Action::Explain(t.id, "waiting: vendor slot/quota")); continue;
        }
        leased.extend(&t.write_scope);
        *per_vendor.entry(route.vendor).or_default() += 1;
        free -= 1;
        actions.push(Action::Start { task: t.id, route });
    }
    actions
}
```

### 6.6 Worker execution

1. **Worktree:** `git -C <repo> worktree add -b codly/<plan>/<task-key> <appdata>/worktrees/<workspace>/<plan>/<task-key> <integration_head>`. Shell out to `git`; it is simpler than libgit2 for worktree, merge and rebase, and it matches what users have. Optionally APFS-clone dependency dirs (`cp -cR`).
2. **Run:** spawn through the adapter with cwd = worktree, a pre-assigned session id, and a role prompt.
   - **Claude** [flags V-local]: `claude -p --output-format stream-json --input-format stream-json --verbose --include-partial-messages --session-id <uuid> --permission-mode acceptEdits --json-schema <worker_report.schema.json> --append-system-prompt "<codly worker rules>"`. Approvals go over the control protocol (sibling report).
   - **Codex:** app-server JSON-RPC (sibling report), or `codex exec --json -C <wt> -s workspace-write --output-schema worker_report.json "<prompt>" < /dev/null`. Resume with `codex exec resume <thread_id> "<feedback>"` [V-local subcommand].
   - **ACP agents (Gemini, OpenCode, …):** `session/new` with cwd = worktree (sibling report).
   - **Deep Agents:** sidecar `run/start` with cwd = worktree.
3. **Worker prompt (the delegation contract):**

```
Task <key>: <title>
Goal: <goal>
Done when (all must hold):
  1. <criterion> ...
You may modify ONLY: <write_scope globs>. You may read anything.
If you must change a file outside that scope, STOP and call codly_report_blocker (or reply with status "blocked").
Verify with: <verify commands>. Run them before you finish.
Commit your work in this worktree when done: git add -A && git commit -m "codly(<key>): <title>"
Final answer: JSON matching the provided schema.
Notes from dependencies: <blackboard notes injected by orchestrator>
```

4. **`worker_report.schema.json`:**

```json
{"type":"object","additionalProperties":false,
 "required":["status","summary","files_changed","checks_run","notes_for_reviewer","scope_exceptions"],
 "properties":{
  "status":{"type":"string","enum":["done","blocked","failed"]},
  "summary":{"type":"string"},
  "files_changed":{"type":"array","items":{"type":"string"}},
  "checks_run":{"type":"array","items":{"type":"object","additionalProperties":false,
     "required":["command","passed"],"properties":{"command":{"type":"string"},"passed":{"type":"boolean"}}}},
  "notes_for_reviewer":{"type":"string"},
  "scope_exceptions":{"type":"array","items":{"type":"string"}}}}
```

The worker's self-report is **advisory**. Codly re-runs the checks (§6.7).

### 6.7 Gate

1. **Verify (Codly-executed):** `tokio::process` in the worktree with a timeout (default 10 min) and env scrubbing. Store the output as an `artifact(kind='test_report')`. On failure, resume the worker session with "Verification failed: `<cmd>` exited `<code>`. Last 80 lines: … Fix and re-run," and increment `attempt`.
2. **Scope:** `git diff --name-only <base_commit>..HEAD` must be ⊆ `write_scope` ∪ declared `scope_exceptions`. Undeclared violations mean an automatic `changes_requested` ("revert edits to X or justify"). Declared exceptions go to the reviewer.
3. **Reviewer run:** a fresh session with **no** worker history. Input: goal, criteria, `git diff base..HEAD` (truncated per file, with full files on request through read tools), verify results.

```json
{"type":"object","additionalProperties":false,"required":["verdict","criteria","issues","summary"],
 "properties":{
  "verdict":{"type":"string","enum":["approve","request_changes","reject"]},
  "criteria":{"type":"array","items":{"type":"object","additionalProperties":false,
     "required":["criterion","met","evidence"],
     "properties":{"criterion":{"type":"string"},"met":{"type":"boolean"},"evidence":{"type":"string"}}}},
  "issues":{"type":"array","items":{"type":"object","additionalProperties":false,
     "required":["severity","file","line","description","suggested_fix"],
     "properties":{"severity":{"type":"string","enum":["blocker","major","minor","nit"]},
       "file":{"type":"string"},"line":{"type":"integer"},"description":{"type":"string"},
       "suggested_fix":{"type":"string"}}}},
  "summary":{"type":"string"}}}
```

**Policy:**

- `approve` requires every criterion `met:true` and **no blocker or major issues**. Otherwise Codly downgrades the verdict to `request_changes`, a deterministic override logged as a decision.
- Issues go back to the worker's **same** session (it keeps its context).
- Nits are recorded but don't block.
- **Filter reviewer comments against the original request** before forwarding, to avoid scope creep loops (Cognition's "communication bridge").

### 6.8 Merge-back queue (serialized)

```bash
# in the integration worktree, one task at a time, topological order then approval time
git merge --no-ff --no-edit codly/<plan>/<task>        # or --squash + commit "codly(<key>): <title>"
# conflict?  → git merge --abort ; task → changes_requested with conflicting paths;
#              resume worker: "rebase onto codly/<plan>/integration@<sha>, resolve, re-verify"
# clean?     → run plan.integration_checks (fast subset per merge; full set at the end)
#              fail → git reset --hard ORIG_HEAD ; task → changes_requested with failure tail
#              pass → task → done ; release leases ; dependents become ready (they branch from new HEAD)
```

**Idempotency:** before merging, check `git merge-base --is-ancestor <task_head> <integration_head>`. If it is already an ancestor, the task is already merged, so mark it done. Never push. At the end, offer three choices: "merge integration into <user branch>", "open a PR", or "keep the branch".

### 6.9 Crash recovery

**At spawn time:**

- **Pre-assign session ids:**
  - `claude --session-id <uuid>` [V-local help].
  - `gemini --session-id <uuid>` [V-local help]. Note: `gemini --resume` takes `latest` or an index; resuming by UUID is unverified, so prefer ACP `session/resume`.
  - Deep Agents `thread_id` (Codly-chosen).
  - **Codex:** persist `thread_id` from the first `{"type":"thread.started","thread_id":…}` line [V-local] (or the app-server response) with `fsync` before processing more.
- Record `pid`, `pgid` (spawn in a new process group) and the process start time.
- Write each raw stdout line to `runs/<id>/raw.ndjson` **before** parsing. Ingest into `event` with `(run_id, seq)` as an idempotency key.

**On startup:**

1. Reconcile git: `git worktree list --porcelain` against `task.worktree_path`, then `git worktree prune`. If a task's worktree is missing, reset the task to `ready` (attempt+1). Check for leftover merges in the integration worktree (`git status`; if `MERGE_HEAD` exists, run `git merge --abort`).
2. For each run in `starting|running|waiting_approval`:
   - Ingest any tail of `raw.ndjson` that was not yet in `event`.
   - If the PID is alive with the same start time, **kill the process group** (v1). Pipes can't be re-attached.
   - Mark the run `lost`.
3. **Resume** each affected task with a new run (`resumed_from_run_id`, same `session_id`):
   - Claude: `claude -p --resume <uuid> "<continuation>"`.
   - Codex: `codex exec resume <thread_id> "<continuation>"`.
   - ACP: `session/resume` or `session/load`, if the agent advertises it.
   - Deep Agents: `run/input` on the same `thread_id`, using the persistent bun:sqlite checkpointer.
   - The continuation prompt: "Codly restarted while you were working on task <key>. Run `git status` and `git diff` in your worktree, then continue toward the acceptance criteria. Do not redo committed work."
4. Pending approvals whose run is `lost` become `expired`. The resumed run re-requests them if it still needs them.
5. **v2 option:** spawn long runs with stdout redirected to the log file (not a pipe) and in their own session (`setsid`), so they **survive an app crash**. Codly then re-attaches by tailing the file. This only works for adapters that don't need stdin for approvals: Claude with `--permission-prompt-tool` pointing at Codly's MCP server, or ACP over a Unix socket. [I]

### 6.10 Explain panel (what to show per object) [I]

**Agent (click a character):**

- Current task (title, goal, criteria with live checkmarks from the reviewer or verify).
- Live plan (latest `Plan` event).
- **Stated rationale.** Only text the agent emitted: assistant message or reasoning *summary* deltas, labeled "agent's own words". **Never synthesized.**
- The last 10 tool calls (kind, target path or command, status, duration).
- Files touched: union of the `git diff --name-only` and `notify` events in its worktree, each tagged in or out of scope.
- Dependencies (blocked-by and blocks).
- Session id and resume lineage.
- Tokens and cost so far, with `cost_source` badges (provider, estimated or list price).
- A **live terminal** (the raw NDJSON pretty-printed, or the PTY for interactive mode).

**Task:**

- The state timeline (each transition, linked to its `decision` row).
- Every attempt with its gate results.
- The reviewer verdict JSON rendered as a checklist.
- Diff and artifacts.

**Decision (every routing, scheduling, gate, merge, retry and approval-policy decision):**

- What was decided and the alternatives with scores.
- **Method** (rule, model or human). For rules, the rule text, e.g. *"waiting: write scope `web/src/auth/**` overlaps running task t2"*. For models, backend, model, version, raw score → calibrated score vs threshold.
- Latency and cost.
- The inputs or features used.
- Later, the outcome, which feeds calibration.

### 6.11 Living World mapping (events → visible states) [I]

| Agent visual state | Derived from (latest events) |
|---|---|
| idle | no running run |
| thinking | `ThoughtDelta` or `MessageDelta` in the last 2 s, no open tool |
| reading / searching | open `ToolStarted{kind: read/search}` |
| typing code | open `ToolStarted{kind: edit/delete}` |
| running a command | open `ToolStarted{kind: execute}` (show the command on a "terminal" prop) |
| delegating | `SubagentSpawned` (spawn a small companion sprite bound to the `ns`) |
| waiting for you | `PermissionAsked` pending (raise hand; the approval card links to the explain panel) |
| blocked | task `blocked`, or deps unmerged with a lease conflict (shows the decision text) |
| reviewing | role = reviewer and running |
| merging | task `merging` |
| celebrating or failing | task `done` or `failed` transition |

### 6.12 Rust crates (versions verified on crates.io today) [V-reg]

| Crate | Version | Use |
|---|---|---|
| `tokio` | 1.54.1 | runtime, `process`, channels |
| `tokio-util` | 0.7.20 | `LinesCodec`, cancellation tokens |
| `process-wrap` | 10.0.1 | process groups and kill-on-drop for child trees (`command-group` 5.0.1 is unmaintained since 2023) |
| `nix` | 0.31.3 | `killpg`, signals |
| `rusqlite` | 0.41.0 | SQLite, bundled (or `sqlx` 0.9.0 if async is preferred) |
| `petgraph` | 0.8.3 | DAG, toposort, critical path |
| `globset` | 0.4.20 | write-scope matching |
| `jsonschema` | 0.60.1 | validate planner, worker and reviewer outputs |
| `schemars` | 1.2.3 | generate the JSON Schemas from Rust structs (single source of truth) |
| `serde_json` | 1.0.154 | |
| `notify` | 8.2.1 | worktree file events |
| `uuid` | 1.29.0 (v7) / `ulid` 3.0.0 | ids |
| `rmcp` | 3.7.0 | Codly's MCP server (`codly_report_blocker`, `codly_post_note`, approvals) |
| `agent-client-protocol` | 3.5.0 | see the sibling report: use the schema crate plus your own transport |
| `git2` 0.21.0 / `gix` 0.90.0 | | optional read-only git queries; shell out to `git` for worktree, merge and rebase |
| `tauri` 2.12.5, `tauri-plugin-shell` 2.5.0 | | app and sidecar resolution |

### 6.13 Vertical-slice build order [I]

1. Event log, the `run` and `event` tables, raw-log-first ingestion, and the Claude adapter only.
2. A single-task flow: worktree → worker → Codly verify → scope check → merge to integration → done. **No planner yet**: the user's request is task t1.
3. Planner plus validation, then 2–3 parallel workers with leases and dependency gating.
4. A reviewer from a second vendor (Codex), with fix loops.
5. Crash recovery (kill and resume) and the explain panel.
6. The Deep Agents sidecar (OpenRouter) as a third worker type, using the same events.
7. The L1 router and the decision log, then calibration once data exists.

---

## 7. Recommendations for Codly (opinionated)

1. **API-key backend = two modes, one event model.**
   - **(a) BYOK for vendor CLIs.** Inject `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`, or the OpenRouter env for Claude Code, into the existing adapters. Ship this first; it costs almost nothing.
   - **(b) The Deep Agents JS sidecar** for OpenRouter, open-weight models and Codly-controlled harness experiments.
   - Do not adopt the Claude Agent SDK or the OpenAI Agents SDK as backends in v1.
2. **Sidecar = Bun-compiled `codly-agent`** (deepagents 1.15.1 pinned exactly, langsmith 0.9.x, `--ignore-scripts`, `--bytecode`), speaking **NDJSON JSON-RPC 2.0** whose `codly/event` payloads deserialize straight into Rust's `AgentEvent`.
   - Use the **v3 `streamEvents`** protocol.
   - Pass model **instances** only.
   - Port `SqliteSaver` to `bun:sqlite`.
   - Turn on `todoListMiddleware`.
   - Use `LocalShellBackend({virtualMode:true, inheritEnv:false})` rooted at the task worktree, with `execute` HITL plus an allowlist through the `when` predicate.
3. **Capture OpenRouter `usage.cost` with a fetch tee.** Mark every cost as `provider`, `estimated` or `list_price` in the UI. Never show list price as money spent on a subscription.
4. **Orchestrator = centralized, deterministic, Rust.**
   - One planner call → a validated DAG.
   - At most 3 parallel workers (per-vendor caps of 2).
   - **Dependents start only after their deps are merged.**
   - **Write-scope leases** with hot-file serialization.
   - Collapse to a single task when parallelism wouldn't pay (§4.8).
5. **Completion is test-gated by Codly, not claimed by the agent.** Run verify, then the scope check, then a **cross-vendor clean-context reviewer**, then the integration checks. Allow at most 2 fix loops per gate before escalating to the user.
6. **Blackboard, not chat.** Agents talk to the orchestrator through a Codly MCP server and to each other only through orchestrator-curated notes. Revisit direct messaging once Claude teams and ACP subagent RFDs stabilize.
7. **Decisions:**
   - L0 rules first.
   - L1 = one structured call **per task** (`claude -p --model haiku --json-schema` on a subscription; Jev or a cheap OpenRouter model with an API key).
   - L2 = human.
   - Log every decision with features and outcome. Calibrate with isotonic recalibration plus a Clopper–Pearson selective threshold per decision type and backend.
   - **Approvals for high-risk categories are policy, never model.**
8. **Recovery = pre-assigned session ids + raw-log-first + kill-and-resume** in v1. Consider detached, file-backed runs in v2.
9. **Pin and contract-test everything that churns:** deepagents, the Claude and Codex CLI output shapes, and ACP. Keep recorded fixtures in `docs/research/fixtures/` and replay them in CI.
10. **Legal and product check (not engineering):** the Agent SDK's statement on third-party use of claude.ai login, plus subscription quota burn from Codly-initiated calls. Give users an "API-key only for background decisions" toggle.

---

## 8. Open questions and things not verified

- The sidecar was **compiled but not executed** (untrusted-code rule). Still open:
  - Runtime correctness of the v3 event mapping, especially subagent namespace attribution for parallel `task` calls.
  - `stream.interrupts` timing.
  - The OpenRouter usage-chunk drop.
  - Next step: record a real fixture with a cheap model.
- Startup time and memory of the Bun binary (with and without `--bytecode`), and macOS notarization with `allow-jit`.
- Whether `claude -p --tools "Read,Grep,Glob" --json-schema …` behaves well for a long planning session: this combination is untested.
- `gemini --resume <uuid>` semantics, and Codex app-server approval flows (see sibling reports).
- The Anthropic API logprobs status, the OpenAI logprobs model list, and Jev's real calibration on Codly's decision types.
- The "~45% single-agent saturation threshold" attributed to the Google paper is not in the abstract, so treat it as unverified.
- The OpenRouter generation-lookup endpoint path, and whether OpenRouter's final usage SSE chunk includes `choices`.

## 9. Sources

**Deep Agents and LangChain:**

- Deep Agents (Python) source and CHANGELOG: https://github.com/langchain-ai/deepagents (f26eb54, 2026-10-25). Files `libs/deepagents/deepagents/graph.py`, `libs/deepagents/CHANGELOG.md`, `profiles/provider/_openrouter.py`, `libs/code/README.md`.
- Deep Agents (JS) source and CHANGELOG: https://github.com/langchain-ai/deepagentsjs (ea12d8e, 2026-10-25). Files `libs/deepagents/src/{agent.ts,types.ts,stream.ts,middleware/*,backends/*,permissions/types.ts}`, `examples/streaming/*.ts`, `libs/acp/{README.md,src/server.ts}`.
- LangChain docs (fetched through the docs MCP today):
  - https://docs.langchain.com/oss/javascript/deepagents/event-streaming
  - /oss/javascript/langgraph/event-streaming
  - /oss/javascript/deepagents/human-in-the-loop
  - /oss/javascript/deepagents/models
  - /oss/javascript/langchain/models
- npm packages inspected: `@langchain/protocol@0.0.20` (`protocol.ts`), `@langchain/openrouter@0.4.19`, `langchain@1.5.19` (`chat_models/universal.js`, `agents/transformers/subagent.js`, `agents/middleware/{hitl,todoListMiddleware}`), `@langchain/langgraph@1.5.2` (`stream/mux.js`, `stream/convert.js`), `@langchain/langgraph-checkpoint(-sqlite)`.

**Providers, SDKs and packaging:**

- OpenRouter usage accounting: https://openrouter.ai/docs/use-cases/usage-accounting
- OpenRouter Claude Code integration: https://openrouter.ai/docs/guides/guides/claude-code-integration
- OpenRouter `typesafe/jev-router`: https://openrouter.ai/typesafe/jev-router
- Claude Agent SDK overview (policy note): https://code.claude.com/docs/en/agent-sdk/overview
- `@anthropic-ai/claude-agent-sdk@0.3.310` (`sdk.d.ts`, `package.json`, `LICENSE.md`).
- `@openai/agents-core@0.21.0`, `@openai/agents-openai@0.21.0`, `@openai/codex-sdk@0.164.0` (README).
- Codex source (`codex-rs/model-provider-info/src/lib.rs`, WireApi): https://github.com/openai/codex (2026-10-23).
- Tauri sidecars: https://v2.tauri.app/develop/sidecar/
- Bun executables: https://bun.com/docs/bundler/executables

**Orchestration evidence:**

- Anthropic, How we built our multi-agent research system (2025-06-13): https://www.anthropic.com/engineering/multi-agent-research-system
- Claude Code agent teams: https://code.claude.com/docs/en/agent-teams
- Claude Code subagents: https://code.claude.com/docs/en/sub-agents
- Anthropic, A harness for every task: dynamic workflows in Claude Code (2026-06-02): https://claude.dev/blog/a-harness-for-every-task-dynamic-workflows-in-claude-code/
- Cognition, Multi-Agents: What's Actually Working (2026-04-22): https://cognition.com/blog/multi-agents-working
- OpenAI Codex best practices: https://learn.chatgpt.com/guides/best-practices (redirected from developers.openai.com/codex/learn/best-practices)
- Google Research, Towards a science of scaling agent systems (2026-01-28): https://research.google/blog/towards-a-science-of-scaling-agent-systems-when-and-why-agent-systems-work/ and arXiv 2512.08296 (v3, 2026-04-08)
- Xu et al., AI Agent Pull Requests on GitHub: Frequency, Structure, and Merge Conflict Rates: https://arxiv.org/abs/2607.04697

**Calibration:**

- Tian et al., Just Ask for Calibration: https://arxiv.org/abs/2305.14975
- Xiong et al., Can LLMs Express Their Uncertainty?: https://arxiv.org/abs/2306.13063

**TypeSafe Jev:**

- Docs: https://docs.typesafe.ai/introduction/quickstart, https://docs.typesafe.ai/primitives, https://docs.typesafe.ai/confidence.md
- Vercel AI Gateway Jev: https://vercel.com/ai-gateway/models/jev
- Community: https://github.com/AbdelStark/awesome-typesafe-jev, https://github.com/BorisLeMeec/jev, https://github.com/dzhng/jevgrep, https://github.com/nassim-arifette/jevgrep, https://github.com/rmax-ai/jevgrep-eval

**Local verification today:**

- `claude --help` (2.1.309), `codex exec --help` (0.161.0), `gemini --help` (0.62.0), `bun build --help` (1.3.14), `man cp`.
- Structured-output probes for `claude -p` and `codex exec`.
- Sidecar PoC compile at `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/sidecar-poc/sidecar.ts`.
