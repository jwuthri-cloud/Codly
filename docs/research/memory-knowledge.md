# Memory and knowledge layer for Codly

*Research report, 2026-10-25. Scope: memory (shared and per-agent), knowledge triples, RAG over memories and docs, boards, messages and 1:1 chats, and how a supervisor combines context. Code retrieval, CLI protocols, orchestration and visuals have their own sibling reports and are only referenced here.*

**Evidence tags** (same convention as the sibling reports):

- **[V-src]** read in source code today.
- **[V-doc]** official documentation fetched today.
- **[V-paper]** paper (arXiv or proceedings).
- **[V-reg]** crates.io registry API, queried today.
- **[V-local]** measured on this machine today (Apple M5 Pro, 48 GB; Rust 1.96; SQLite 3.51.0 through Python's `sqlite3`), synthetic data only.
- **[Claim]** an author's or vendor's claim I could not check independently.
- **[I]** my inference or design proposal.

**Method.** Munder Difflin ("MD") was read from the shallow clone at `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/munder-difflin` (commit `d3460de`, 2026-10-25; origin `github.com/chaitanyagiri/munder-difflin`, also listed as `github.com/HarnessMD/munder-difflin`). MemPalace, the Python memory engine MD shells out to, was shallow-cloned read-only next to it (`/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/mempalace`, commit `84582fb`, 2026-10-25), because MD's memory can't be described without it. No script or binary from either repository was run, and text inside them was treated as data. Everything else comes from primary pages fetched today. No paid LLM prompt was run. The two local micro-benchmarks (std-only Rust, Python stdlib `sqlite3`) live in `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/`.

**Related reports:** [context-engineering.md](./context-engineering.md) (code retrieval, the `codly-mcp` sidecar, the pointer pack), [agents-backend-orchestration.md](./agents-backend-orchestration.md) (SQLite data model, task board, cheap CLI calls), [cli-claude-codex.md](./cli-claude-codex.md), [cli-others-acp.md](./cli-others-acp.md), [design-world.md](./design-world.md), [munder/](./munder/).

---

## 0. TL;DR

1. **MD's memory is markdown files plus an external Python index, not a database.** Each agent appends to its own `hive/agents/<id>/memory.md`. Every 10 minutes the app shells out to `mempalace mine` to index the files that changed into one shared ChromaDB "palace", one *wing* per agent, embedded with all-MiniLM-L6-v2. That is what the "memory · minilm" badge means. Agents recall with `mempalace search` and `mempalace wake-up` through Bash, taught by one line in their system prompt. There is no MCP. "Private" means single-writer authorship only: any agent can search any wing. [V-src]
2. **MD's "Knowledge Graph" holds no triples.** It is a keyword-scored chunk store (`index.jsonl`, term frequency plus title and phrase boosts) of documents the user ingests, queried through a `kg` CLI. The "memory graph" tab draws who messaged whom, plus bold or heading phrases that at least two agents' memory files share. Real subject–predicate–object triples with validity windows exist only inside MemPalace's own SQLite graph, which MD never connects: it uses MemPalace's CLI, not its MCP tools. [V-src]
3. **MD's other pieces:**
   - one shared kanban (`tasks.json`, maintained by the orchestrator, with human Q&A on cards);
   - a blackboard (`board.md`, the orchestrator is its only writer);
   - file mailboxes with FIPA-style speech acts and a hop cap, and a Stop hook that keeps an agent working while it has mail;
   - a Haiku "reflector" that condenses oversized `memory.md` files behind a verify gate.

   There are no per-agent kanbans in the open-source code. "1:1" is a Pro feature that holds the orchestrator's deliveries while you talk to an agent. [V-src][V-doc]
4. **What the 2025–2026 evidence says works:**
   - Keep the raw record and retrieve it well, with lexical and dense search combined.
   - Never let an LLM rewrite memory destructively. Mem0 switched to ADD-only writes in 2026, and ACE measured "context collapse" from full rewrites.
   - Check code-related memories against the current code before using them. GitHub Copilot Memory stores citations and checks them just in time; its pull-request merge rate rose from 83 % to 90 %.
   - Plain file-like tools that agents already know beat clever ones. Letta scored 74 % on LoCoMo with plain file tools.
   - "Lessons" distilled from successes *and* failures help coding agents. ReasoningBank gained 3.4 to 4.6 points on SWE-bench Verified.

   Memory benchmarks are noisy and often report retrieval recall rather than answer accuracy: MemPalace has 96.6 % R@5 but about 67 % QA accuracy. [V-paper][V-doc][Claim]
5. **Recommended storage: one SQLite database, with no graph database and no Python.** It holds memories, provenance, embeddings (BLOB), FTS5, a bi-temporal knowledge graph (entities and edges), boards and channels. Local measurements:

   | Query | Data | p50 |
   |---|---|---|
   | FTS5 BM25 | 100k memories | 0.65 ms |
   | Brute-force cosine, 1 thread, f32 | 100k × 384-d vectors | 4.9 ms |
   | Same, int8 | 100k × 384-d vectors | 2.4 ms |
   | 2-hop graph traversal, two-arm recursive CTE | 250k edges | 0.15 ms |
   | Same traversal, naive OR-join | 250k edges | 80 ms to 1.2 s |

   Kuzu was archived in October 2025, and Graphiti has deprecated its Kuzu driver. [V-local][V-reg][V-doc]
6. **Recommended embedder: fastembed 7.5.0, with bge-small-en-v1.5 as the model.** fastembed runs ONNX Runtime through `ort =2.0.0-rc.15`. The model has 33M parameters and 384 dimensions, is MIT-licensed, and scores 51.68 on MTEB retrieval, against 41.95 for MiniLM. Published Apple Silicon numbers (MiniLM, M4 Max, one short text):

   | Backend | p50 |
   |---|---|
   | ort | 1.10 ms |
   | fastembed | 1.71 ms |
   | candle | 8.15 ms |

   int8 quantization brings no speedup there. model2vec-rs is a fast fallback, but static models trail on retrieval. For code, BM25 alone beats the 16M static code model. [V-doc][V-reg][Claim]
7. **Most write volume needs no LLM.** Four paths:
   - (a) deterministic triples taken from tool events, for example "agent A edited `src/x.ts` for task T" or "T ran `pnpm test`, which failed";
   - (b) a structured `task_report` that the worker writes as its last step, with decisions, lessons and citations;
   - (c) explicit `memory_write` calls;
   - (d) user pins.

   LLM extraction stays opt-in, batched and rate-limit aware. The call is `claude -p --model haiku --json-schema …`; the sibling probe measured about 3 s per call, and Anthropic's October 2026 note says this usage still counts against the subscription. [I][V-doc]
8. **Two read paths: a per-turn memory pack and MCP tools.** A Rust "context broker" builds a budgeted pack: by default at most 800 tokens for a worker and 2,000 for the supervisor. It holds relevant memories the agent has *not* seen yet, plus deltas (new messages, board changes, decisions), and it logs exactly what each agent received. The `codly-mcp` sidecar that the code-retrieval report already proposes gains `memory_search`, `memory_write`, `kg_query`, `board`, `message` and `task_report`. [I]
9. **The supervisor combines context through the blackboard and structured reports, not transcripts.** Each worker receives:
   - a four-part contract (objective, output, tools, boundaries), which MD and Anthropic both recommend;
   - its memory pack;
   - the decisions of sibling tasks that touch the same entities.

   The supervisor receives reports, blockers, memories awaiting review and fleet statistics, and pulls details only when it needs them. [V-doc][I]
10. **First vertical slice:**
    - the schema;
    - hybrid memory search;
    - the `memory_write`, `task_report` and `memory_search` tools;
    - the per-turn pack with an injection log;
    - a rule-based activity graph;
    - a memory browser with provenance;
    - shared and per-agent boards;
    - user ↔ agent 1:1 chat;
    - library and whiteboard animations driven by real tool calls.

    Defer LLM extraction, contradiction detection, graph visualisation with time travel, approximate-nearest-neighbour indexes and importing the CLIs' own memory. [I]

---

## 1. Munder Difflin: how its memory and knowledge work

### 1.1 Map of the code

| Feature | Mechanism | Where in the MD repo | Tag |
|---|---|---|---|
| Private memory | `hive/agents/<id>/memory.md`, plain markdown. The agent is told to read it at the start of a task and to append durable facts. | `src/main/hive.ts` (`injectedPrompt`, `PROTOCOL_MD`) | [V-src] |
| Shared semantic recall | MemPalace CLI. One palace lives under the harness home. `mempalace mine <agentDir> --wing <id> --agent <id>` runs every 10 min for files that changed. Agents run `mempalace search "<q>" [--wing <id>]` and `mempalace wake-up` through Bash. | `src/main/memory.ts`, `src/main/palaceReap.ts` | [V-src] |
| Embeddings | `minilm` by default (all-MiniLM-L6-v2, 384-d, through chromadb's `ONNXMiniLM_L6_V2`) or `embeddinggemma`. CPU is forced on macOS. | `memory.ts`; MemPalace `mempalace/embedding.py` | [V-src] |
| Consolidation | `MemoryReflector` condenses an oversized `memory.md` with Haiku into pinned facts, one rolling summary and the 12 newest sections, behind a verify gate. | `src/main/reflect.ts` | [V-src] |
| "Knowledge Graph" | Documents the user ingests become 1,200-character chunks in `index.jsonl`, scored by keywords. Agents run `node "$KG_CLI" search`. | `src/main/kg-core.cjs`, `knowledge.ts`, `resources/kg.cjs`, `docs/design/knowledge-graph.md` | [V-src] |
| Memory graph tab | SVG force layout with agent nodes, message edges and "topic" nodes. A topic is a heading or bold phrase that at least 2 agents' memory files share. | `MEMORY_GRAPH_SPEC.md`, `src/renderer/src/components/memoryGraph/*` | [V-src] |
| Shared kanban | `hive/tasks.json`: statuses todo, doing, blocked and done, plus assignee, dependencies, priority and `humanQA`. | `TasksKanban.tsx`, `hive.ts` | [V-src] |
| Blackboard | `hive/board.md`. The orchestrator is its only writer; other agents `propose` changes. | `PROTOCOL_MD` in `hive.ts` | [V-src] |
| Messaging | One JSON file per message in `outbox/`. The main process moves it into the recipient's `inbox/`. Messages carry speech acts and a hop cap, and a Stop hook delivers them. | `hive.ts`, `HIVE.md` | [V-src] |
| Human ↔ agent | The Threads panel lets the human reply as sender `human`. The "Ask me" board collects card questions. Pro adds "hold a 1:1". | `ThreadsPanel.tsx`, `AskMeTab.tsx`; `docs/pro/index.html` | [V-src][V-doc] |
| Supervisor context | The orchestrator's prompt tells it to read `fleet.json` (tokens, cost, status, last tool, breaker, inbox backlog) and `registry.json`, and to keep `board.md` and `tasks.json` current. It also gives a 4-part dispatch contract and the rule "pass references … not pasted content". | `hive.ts` (`godLine`) | [V-src] |
| Persistence | The hive is a git repo that only the main process commits to. SQLite (`better-sqlite3`) holds only key-value state and command history. | `HIVE.md`, `src/main/db.ts` | [V-src] |

### 1.2 Private versus shared memory

- **Per-agent workspace.** Each agent directory holds:
  - `identity.md`, written by the harness and read-only for the agent;
  - `memory.md`;
  - `inbox/` and `inbox/.done/`;
  - `outbox/`;
  - `cursor.json`.

  The design rule is "single-writer-per-file": an agent writes only inside its own directory ([HIVE.md](https://github.com/chaitanyagiri/munder-difflin/blob/main/HIVE.md) §2). Memory files follow a loose convention: dated `## <date> — <title>` sections, bold key terms and bullets (`MEMORY_GRAPH_SPEC.md` §5). [V-src]
- **How agents learn the protocol.** Claude agents receive a block through `--append-system-prompt`:
  1. "At the START of a task, read … memory.md and EVERY file in … inbox."
  2. "Record durable facts, decisions, and context by appending to … memory.md."
  3. Message other agents through `outbox/`.
  4. "At the END of a task, append what you learned to memory.md."

  Memory therefore depends entirely on the agent following instructions. Nothing is extracted automatically. [V-src `hive.ts` `injectedPrompt`]
- **Shared recall.** All agents point `MEMPALACE_PALACE_PATH` at one palace. A search sweeps every wing unless `--wing <id>` narrows it. MD's blog sums it up as "private authorship … shared recall": any agent can read any agent's notes. There is no read access control. [V-src `memory.ts`; [blog post](https://munderdiffl.in/blog/how-agents-remember-semantic-memory/)]
- **Shared static context.** The hive root holds `board.md`, `tasks.json`, `registry.json`, `fleet.json`, `log.jsonl`, `PROTOCOL.md` and `COMMANDS.md`. [V-src]
- **Prompt caching.** The injected prompt is kept "volatile-free": no per-turn data, and only paths that are stable for the install, so Claude's prompt cache prefix holds. Codly should copy this. [V-src `knowledge-graph.md` §5.1, `hive.ts` comments]

### 1.3 The semantic layer (MemPalace) and its failure modes

**Mining.** `MemoryManager.mineNow()` walks `hive/agents/*` and skips any `memory.md` whose mtime hasn't changed. It runs `mempalace mine` one agent at a time, because the palace allows a single writer. Settings: [V-src `memory.ts`]

- interval `MINE_INTERVAL_MS = 600_000`, which is 10 min (a June blog post still says 3 min);
- a hard timeout of 10 min per mine;
- a back-off of up to 30 min while the palace is "quarantining".

Before each mine it writes a `.gitignore` into the agent directory that excludes `settings.json`, `cursor.json`, `inbox/`, `outbox/` and `.codex/`. The blog post ["Don't Index the Config"](https://munderdiffl.in/blog/keep-agent-semantic-memory-clean/) explains why: the wake-up digest was being swamped by configuration files.

**Recall.**

- `mempalace search "<q>" --results N [--wing id]` returns ranked snippets.
- `mempalace wake-up` returns a digest built from MemPalace's "4-layer stack": L0 identity (~100 tokens) and L1 "essential story" (~500–800 tokens). The source states "Wake-up cost: ~600-900 tokens (L0+L1)". [V-src `mempalace/layers.py`]

The app calls the CLI as child processes with a 120 s timeout. They used to be `spawnSync` calls that blocked Electron's main process for up to two minutes on a cold model load (code comment). In the app, then, recall means spawning a Python process that loads an ONNX model. I did not measure that latency.

**Embedding.** `MEMPALACE_EMBEDDING_MODEL` is `minilm` (the default) or `embeddinggemma`. On macOS MD forces `MEMPALACE_EMBEDDING_DEVICE=cpu`. The reason, in a code comment dated 2026-08-16: CoreML ran the quantized EmbeddingGemma graph with fp16 partitions that overflowed, so "EVERY vector comes back NaN" and Chroma rejected every write. MiniLM never reaches CoreML anyway, because chromadb removes that execution provider. [V-src `memory.ts`]

**Optional dependency.** If the `mempalace` CLI isn't on PATH (it is installed through uv or pip), the semantic layer silently does nothing and markdown memory still works. MD's prerequisites page offers to install it. [V-src; [munder/features.md](./munder/features.md)]

**Failure modes documented in MD's own comments.** [V-src]

1. The CoreML NaN bug described above.
2. A ChromaDB HNSW segment quarantine loop. MemPalace renames a suspect segment and never deletes the copy. A 100-vector batch never reaches Chroma's sync threshold of 1000, so the health check fails again every few minutes: "A user on Discord passed 100GB inside 8 hours this way". MD ships `palaceReap.ts` to delete the copies.
3. `mempalace init` ends in an interactive prompt that `--yes` doesn't cover, so a spawned child would hang. MD skips `init`.
4. Writers must be serialized.

**MemPalace itself** ([repo](https://github.com/MemPalace/mempalace)):

- It stores conversation text verbatim: "It does not summarize, extract, or paraphrase". Its wings, rooms and drawers are metadata filters. ChromaDB is the default backend; SQLite-exact, Qdrant, pgvector and a native Rust engine are pluggable.
- Its knowledge graph lives in SQLite: an `entities` table, and a `triples` table with `valid_from`, `valid_to`, `confidence`, `source_closet`, `source_file`, `source_drawer_id` and `adapter_name`. `add_triple` only de-duplicates identical open triples. `supersede()` closes the old fact and opens the new one "at a single shared instant", so an as-of query at the boundary never returns both. Queries cover entity, relationship, timeline and as-of. [V-src `mempalace/knowledge_graph.py`]
- An MCP server exposes about 30 tools: `kg_add`, `kg_query`, `kg_invalidate`, `kg_supersede`, `kg_timeline`, search, diary, drawers and more. [V-src `mempalace/mcp_server/*`]
- A regex extractor tags decisions, preferences, milestones, problems and "emotional" passages without any LLM. [V-src `general_extractor.py`]

**MemPalace benchmarks.**

- Its README claims "96.6% R@5 raw on LongMemEval — zero API calls" [Claim].
- An independent critique ([Dey & Viradecha, arXiv:2604.21284](https://arxiv.org/abs/2604.21284), April 2026) [V-paper] finds that:
  - the figure is session-level recall_any@5, "the most generous variant of recall";
  - it is reproducible with bare ChromaDB and the default MiniLM;
  - QA accuracy is about 67.2 %;
  - room-based boosting scores 89.4 %, below raw search;
  - the knowledge graph is a flat, single-hop triple store with no contradiction detection;
  - the lossy AAAK compression mode drops recall to 84.2 %.
- MD's own design doc says MemPalace's "public benchmarks are overstated per independent audit". [V-src `HIVE.md`]

### 1.4 The "Knowledge Graph" and the triples question

**`kg-core.cjs`** is pure JavaScript, so agents can run it out of process with plain `node`; `better-sqlite3` is compiled for Electron's ABI. [V-src]

- **Ingestion** copies the original file, then extracts text:
  - text formats are taken verbatim;
  - PDFs go through `pdftotext` when it is on PATH;
  - images are reduced to title, caption, tags and filename.

  The text is split into chunks of about 1,200 characters with a 150-character overlap, and one JSON line per chunk is appended to `index.jsonl`.
- **Search** tokenizes into lowercase alphanumerics with a small stop-list. Each chunk scores Σ(1+ln(1+tf)) over the query terms, plus 2 per title hit, plus 0.5 per distinct matched term, plus 5 for an exact phrase match. It returns the top 8.
- The feature is **off by default**. Agents get read-only access through a prompt line.
- The design's forward path is FTS5, then embeddings, then a "true graph layer", then MCP. **None of it is built.** [V-src `docs/design/knowledge-graph.md` §8]

**Triples.** MD has no subject–predicate–object store, no entity resolution and no temporal facts. MemPalace has all three, but MD only calls `mine`, `search` and `wake-up`. HIVE.md records the decision: "MemPalace CLI (not MCP, by decision)", with MCP listed as a possible future upgrade. If a user wants "triplet knowledge", they must install MemPalace's MCP server into their agents themselves. [V-src]

**Memory graph tab.** [V-src `MEMORY_GRAPH_SPEC.md`, `extractTopics.ts`]

- Nodes are agents, plus "broadcast" and "human" pseudo-nodes.
- Edges aggregate the last 200 log messages, coloured by speech act.
- An optional layer shows up to 24 "topics": `##` heading tails and `**bold**` spans that at least 2 agents share.
- A hand-rolled SVG force layout draws it, polling every 5 s.

The spec says it plainly: "heuristic, not semantic".

### 1.5 Consolidation: the reflector

`reflect.ts` "finishes the janitor's job". [V-src]

| Setting | Value |
|---|---|
| Budget per agent | 128 KB |
| Triggers | over 50 % of the budget, or more than 50 `##` sections in a file over 16 KB |
| Scan interval | 30 min |
| Recent sections kept verbatim | 12 |

The evicted sections and the previous summary are condensed into **one** bounded section of at most 1,500 words, which keeps "every decision, root cause, protocol, file path, commit SHA". New durable facts are "hoisted" into a pinned block that is never condensed.

- **Model:** `claude-haiku-4-5`, run through a *hidden interactive PTY* session rather than `claude -p`. A code comment says this was to stay on interactive plan quota instead of a separate Agent SDK credit pool announced for 2026-06-15. Anthropic has since paused that change: its help page, updated 2026-10-07, says `claude -p` and Agent SDK usage "still draw from your subscription limits" ([support article](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)). [V-doc]
- **Safety:** a lossless backup comes first. Then a verify gate checks that:
  - the result parses into the three regions;
  - every old pinned line survives;
  - the new file is at least 5 % smaller;
  - the kept sections are byte-identical.

  If all checks pass, the new file is swapped in atomically. Any failure leaves the file untouched and logs `condense-abort`.

### 1.6 Kanban, blackboard, messages, 1:1

**Shared kanban.** `tasks.json` holds cards with these fields: `id`, `title`, `description`, `assignee`, `status` (todo, doing, blocked or done), `dependsOn[]`, `priority`, `createdAt` and `humanQA[]` (entries `{q, a, askedAt, answeredAt, dismissedAt}`). [V-src `TasksKanban.tsx`, `hive.ts`]

- The orchestrator must set `assignee` at dispatch and never clear it.
- A blocked card with an open question shows up on the "ASK ME" board.
- The human's answer is written into the card and also arrives as an inbox message to the orchestrator.

**Per-agent lists.** There is no per-agent kanban in the open code. When Claude calls its `TodoWrite` tool, the avatar walks to the "board" station (`useHive.ts`), but this is only a visual cue. [V-src]

**Messages.** [V-src `HIVE.md` §4–5]

- Fields: `id`, `conversation`, `in_reply_to`, `from`, `to` (an agent, `god`, `broadcast` or `human`), `act` (request, inform, propose, query, agree, refuse or done), `subject`, `body`, `hops`, `requires_reply`, `needs_human` and `created_at`.
- Only request, query and propose expect a reply.
- The hop counter is capped.
- A message `to:"human"` goes to the orchestrator.
- Delivery happens at the end of a turn. A `Stop` hook returns `{"decision":"block","reason": <messages>}` to keep the agent working, guarded by `stop_hook_active`.

**1:1.**

- Open-source: the Threads panel groups an agent's inbox by conversation and lets the human reply inline. [V-src]
- Pro, per its page: each agent has a room with the conversation and its terminal. "Send" queues a message; "Steer lands on its next turn"; "hold a 1:1 so the orchestrator sends it nothing while you talk." [V-doc [Pro page](https://munderdiffl.in/pro/)]

### 1.7 Supervisor context

The orchestrator ("Michael", the "god" agent) is an LLM that reads files. Its prompt asks it to: [V-src `hive.ts`]

- keep awareness of `fleet.json`, `registry.json` and the task board;
- route work to existing agents before spawning new ones, using the `ctx NN%` context-occupancy tag in the live roster;
- write each dispatch as "a 4-part contract … (1) OBJECTIVE … (2) OUTPUT … (3) TOOLS … (4) BOUNDARIES";
- "Pass references (file paths, message ids, board sections), not pasted content".

Nothing is merged automatically. A worker knows only what the orchestrator puts in its inbox, its own `memory.md` and whatever it finds in the palace.

### 1.8 Pro edition (closed source)

The Pro page describes features that are not in the public code:

- Memory "becomes a graph of agents, tickets and notes".
- "Two searches, labelled: exact matches, and by meaning."
- Answers come in three shapes: tickets, agents and notes.
- "Send the results to an agent with a prompt."
- A sidebar "Memory" section shows "412 topics" in the mock-up.

I could not verify how any of this works. [V-doc [Pro page](https://munderdiffl.in/pro/)]

### 1.9 What to copy and what to beat

| MD idea | Keep? | Codly's version [I] |
|---|---|---|
| Markdown-first, human-readable memory | Keep it as a *view* | SQLite is canonical. Every memory renders as a readable card and exports to markdown. |
| Single writer per file | Keep | One Rust writer task. Agents write only through MCP. |
| Graceful degradation | Keep | FTS5 is always on. Embeddings and LLM jobs are optional. |
| Volatile-free system prompt (prompt cache) | Keep | Per-turn data goes in the turn text, never in the system prompt. |
| 4-part contract; references, not content | Keep | The contract is a task field. Packs carry ids. |
| Speech acts and hop cap | Keep | `act` and `hops` columns. |
| Stop hook drains the inbox | Keep for Claude | Generic delivery at turn boundaries for every CLI. |
| "Verify, don't trust" for LLM rewrites | Keep | And avoid destructive rewrites altogether (§2.9). |
| External Python index with up to 10 min of indexing lag | Drop | Embeddings in process; a memory is searchable a few ms after it is written. |
| Vector-only recall for memory; keyword-only "KG" for documents | Drop | Hybrid FTS5, vector and entity search fused with RRF for both. |
| "Private" means authorship only | Drop | Read access control per scope: global, project, agent, task. |
| Untyped notes without provenance | Drop | Kinds, scopes, citations and links to the events that produced each memory. |
| A keyword document store called "Knowledge Graph" | Drop | Bi-temporal triples, plus RAG over documents. |
| One shared kanban | Extend | A shared board, per-agent boards, and mirrors of each CLI's own to-do list. |

---

## 2. State of the art in agent memory (2025–2026)

### 2.1 Overview

| System | How memories are written | Store | How they are read | Strongest evidence | Caveats |
|---|---|---|---|---|---|
| **Mem0** (2025 paper) | LLM extraction, then LLM reconciliation (add, update, delete); optional graph variant | vector store, optional graph | similarity search | LoCoMo LLM-judge score: +26 % relative to OpenAI's memory; −91 % p95 latency and >90 % fewer tokens than full context [V-paper] | Zep and Letta both dispute the numbers [Claim] |
| **Mem0** (2026 algorithm) | one LLM pass, **ADD-only** (nothing overwritten or deleted), entity linking | vectors plus an entity layer | semantic, keyword and entity signals fused by rank | LoCoMo 92.5, LongMemEval 94.4, BEAM-1M 64.1, at about 7k tokens per query [Claim] | measured on the managed platform; judge model not named |
| **Zep / Graphiti** | LLM extraction of entities and facts per episode, deduplication, time extraction, invalidation of contradicted edges | Neo4j, FalkorDB or Neptune (Kuzu deprecated) | cosine + BM25 + breadth-first graph search; RRF, MMR or cross-encoder reranking | LongMemEval_S with gpt-4o: 60.2 % → 71.2 %, latency 28.9 s → 2.6 s, context 115k → 1.6k tokens [V-paper] | several LLM calls per episode; needs structured output |
| **Letta / MemGPT** | the agent edits memory blocks; archival store; sleep-time agents; since 2026, git-backed markdown files | Postgres, or files plus git | blocks always in context; tools for the rest | 74.0 % on LoCoMo with only file tools (gpt-4o-mini) [V-doc] | vendor-run benchmark |
| **LangMem / LangGraph store** | tools on the hot path, or reflection in the background | namespaced store with a vector index | search by namespace, query and filter | none (it is a framework) | |
| **Cognee** | add → cognify (LLM or local GLiNER extraction) | relational + vector + graph | routed search across graph, vector or code | BEAM 0.79 at 100k tokens [Claim] | |
| **A-MEM** | LLM builds a note (keywords, tags, context), links it, and evolves neighbouring notes | notes plus embeddings | top-k cosine | LoCoMo multi-hop F1 45.85 vs 18.41 [V-paper] | 13 LLM calls per response and lower scores when MemoryOS re-ran it [V-paper] |
| **MemoryOS** | short-term → mid-term → long-term, promotion by "heat" | three tiers | segment, then page | LoCoMo average F1 36.23 vs 29.13 for MemGPT (gpt-4o-mini) [V-paper] | |
| **MemPalace** | stores text verbatim; regex extraction; triples through MCP | ChromaDB (or others), SQLite graph | vector search with metadata filters; wake-up layers | LongMemEval R@5 96.6 % (retrieval only) [Claim]; about 67.2 % QA per the critique [V-paper] | claims overstated (§1.3) |
| **GitHub Copilot Memory** | agent calls `store_memory`: subject, fact, **citations**, reason | service, scoped to the repository | recent memories in the prompt, citations checked just in time | PR merge rate 83 % → 90 % (vendor A/B test) [Claim] | unused memories deleted after 28 days [V-doc] |
| **Claude Code** | CLAUDE.md (written by people) and auto memory (typed notes Claude writes) | markdown files | `MEMORY.md` index (first 200 lines or 25 KB) at start; topic files on demand | none published | one directory per repository, shared across worktrees |
| **Codex** | AGENTS.md, plus "memories": background extraction and consolidation | files under `~/.codex/memories/` | injected into new sessions | none published | off by default |
| **ReasoningBank** | after each task an LLM judges success or failure and keeps at most 3 strategy items | embeddings | top-1 item added to the system prompt | SWE-bench Verified 34.2 → 38.8 (Gemini 2.5 Flash), 54.0 → 57.4 (Pro) [V-paper] | |
| **ACE** | generator, reflector and curator; small "delta" bullets merged by deterministic code | itemized playbook | whole playbook in context | +10.6 % on agent tasks, +8.6 % on finance; adaptation latency −86.9 % [V-paper] | |

### 2.2 Mem0

- **2025 design** ([Chhikara et al., arXiv:2504.19413](https://arxiv.org/abs/2504.19413), April 2025). The abstract describes extraction, consolidation and retrieval of "salient information", plus a graph variant (Mem0g) about 2 % better overall. On LoCoMo it reports a 26 % relative gain on the LLM-as-a-judge score over OpenAI's memory, 91 % lower p95 latency and more than 90 % fewer tokens than full context. [V-paper]
- **Disputes.**
  - Zep says Mem0 configured Zep badly and that Zep's corrected LoCoMo score is 75.14 % ± 0.17 ([Zep blog](https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/)). [Claim]
  - Letta reports 74.0 % with plain file tools against Mem0's best published 68.5 %, and says Mem0 did not explain how its numbers were computed ([Letta](https://www.letta.com/blog/benchmarking-ai-agent-memory)). [Claim]
- **2026 algorithm** ([Mem0 blog](https://mem0.ai/blog/mem0-the-token-efficient-memory-algorithm), April 16, 2026, updated September 28, 2026). [Claim]
  - It uses **one** extraction call, and every fact becomes a new record: "Nothing is overwritten or deleted". Facts the agent states are kept with the same weight as the user's.
  - Each memory is analysed for entities. The entities go into a lookup layer that links memories about the same thing.
  - Retrieval runs three signals in parallel: semantic similarity, keyword matching with verb normalization, and entity matching. Results are fused by rank.
  - Mem0 explains the switch by the old reconciliation step: overwrites "sometimes erased key details", and deletes removed information that later mattered.
  - Results, old → new: LoCoMo 71.4 → 92.5; LongMemEval 67.8 → 94.4; BEAM at 1M tokens 64.1 and at 10M tokens 48.6; about 6.7k–7.0k tokens per query.
  - The weakest categories are contradiction resolution (35.7 at 1M, 32.5 at 10M) and temporal reasoning at 10M (16.3).
  - Caveats: the scores come from the managed platform, the judge is not named, and judge noise is about ±1 point.
- **Lesson for Codly:** don't reconcile destructively. Append, link entities, and spend the effort on retrieval. [I]

### 2.3 Zep and Graphiti

- **Architecture** ([Rasmussen et al., arXiv:2501.13956](https://arxiv.org/html/2501.13956), January 2025). [V-paper]
  - Three tiers: raw *episodes*, *entities* with *fact* edges between them, and *communities* (clusters with summaries).
  - Every edge carries four timestamps: `t_valid` and `t_invalid` (when the fact held) and `t'_created` and `t'_expired` (when the system learned it or retracted it). When an LLM finds a new fact that contradicts an old one over the same period, it sets the old edge's `t_invalid` to the new edge's `t_valid`.
  - Extraction sees the current message plus the previous 4.
  - Entity resolution embeds each name (1024-d), gathers candidates by cosine similarity and full-text search, and asks an LLM to decide.
  - Facts are deduplicated only within the same entity pair.
  - Retrieval combines cosine, BM25 and breadth-first search, then reranks with RRF, MMR, episode-mention counts, graph distance or a cross-encoder.
- **Results.** [V-paper]

  | Model | Memory | Accuracy | Latency | Context tokens |
  |---|---|---|---|---|
  | gpt-4o-mini | full context | 55.4 % | 31.3 s | 115k |
  | gpt-4o-mini | Zep | 63.8 % | 3.20 s | 1.6k |
  | gpt-4o | full context | 60.2 % | 28.9 s | 115k |
  | gpt-4o | Zep | 71.2 % | 2.58 s | 1.6k |

  These are LongMemEval_S results. One category got worse: single-session-assistant fell from 94.6 % to 80.4 % with gpt-4o. On the DMR benchmark Zep scores 94.8 % against MemGPT's 93.4 %.
- **Graphiti today** ([repo](https://github.com/getzep/graphiti)). [V-doc]
  - Backends: Neo4j 5.26, FalkorDB 1.1.2 (with an embedded "FalkorDB Lite" that needs Python 3.12+), and Amazon Neptune.
  - **Kuzu is deprecated** because "the upstream project is unmaintained".
  - Structured output is required: smaller models may "produce incorrect schemas and ingestion failures".
  - The MCP and REST servers are reference implementations, with no authentication and no prompt-injection protection.
- **Deterministic entity deduplication before the LLM.** Graphiti's `dedup_helpers.py` runs these steps in order: [V-src]
  1. Exact match on the lowercased name with whitespace collapsed.
  2. An entropy gate: names under 6 characters with fewer than 2 tokens, or with Shannon entropy below 1.5, skip fuzzy matching and go to the LLM.
  3. Fuzzy match: 3-gram shingles, MinHash with 32 permutations, LSH bands of 4, and acceptance at Jaccard similarity ≥ 0.9.
  4. Anything still unresolved goes to the LLM.

  Codly can reuse this as-is (§3.4).
- **Edge-extraction prompt** (`prompts/extract_edges.py`). [V-src]
  - Edges may only connect entities from the provided list.
  - Relation types are `SCREAMING_SNAKE_CASE`.
  - Facts keep concrete specifics.
  - An ongoing fact, or a future plan, takes the episode's time as `valid_at`.
  - Dates are never inferred.

### 2.4 Letta and MemGPT

- **Memory blocks** ([docs](https://docs.letta.com/guides/agents/memory-blocks)) have a `label`, a `description`, a `value` and a character `limit`, and can be `read_only`. They "are simply prepended to the agent's prompt in an XML-like format". One block can be attached to several agents, which gives shared memory where the last write wins. [V-doc]
- **Sleep-time compute** ([Lin et al., arXiv:2504.13171](https://arxiv.org/abs/2504.13171)) means thinking about a context before any query arrives. It needs about 5× less test-time compute for the same accuracy, gains up to 13 % and 18 % on two benchmarks, and costs 2.5× less per query when several queries share a context. [V-paper]
- **"Is a filesystem all you need?"** ([Letta, August 2025](https://www.letta.com/blog/benchmarking-ai-agent-memory)): 74.0 % on LoCoMo with gpt-4o-mini and only `grep`, `search_files`, `open` and `close`. Letta's argument is that remembering depends on whether the agent retrieves the right thing, and agents are good with file tools because they were trained on coding. [V-doc][Claim]
- **Context Repositories** ([Letta, February 12, 2026](https://www.letta.com/blog/context-repositories)). Letta Code memory is now a git-backed directory of markdown files. [V-doc]
  - The file tree is always in the system prompt, and `system/` files are fully loaded.
  - Every file has YAML frontmatter that describes it.
  - Reflection runs as a background process in its own git worktree and merges back.
  - First-run initialization fans subagents out over past Claude Code and Codex histories.
  - "Defragmentation" reorganizes memory into about 15–25 files.

  The MemGPT authors themselves converged on files, git and background reflection. That is close to MD's design, but with real concurrency control.

### 2.5 LangMem and the LangGraph store

- **Memory types** ([LangMem guide](https://langchain-ai.github.io/langmem/concepts/conceptual_guide/)). [V-doc]
  - *Semantic* memory is either a *collection* of records or one *profile* updated in place.
  - *Episodic* memory keeps successful interactions as examples.
  - *Procedural* memory is the agent's instructions, refined through prompt optimization.

  Memories form either on the "hot path" (immediate, but adds latency) or in the "background" (no latency, delayed). Over-extraction hurts precision and under-extraction hurts recall. Retrieval should weigh similarity, importance and "strength", which grows with recency and use.
- **Store API** ([docs](https://docs.langchain.com/oss/python/langgraph/stores.md)): namespaces are tuples. `put(namespace, key, value, index=[fields] | False)`, `search(namespace_prefix, query=, filter=, limit=10)`, and an index config `{embed, dims, fields}`. [V-doc]
- **Supervisor library** ([langgraph-supervisor](https://github.com/langchain-ai/langgraph-supervisor-py)). `output_mode="full_history"` or `"last_message"` decides how much of a worker's trace reaches the supervisor. The repository is archived, and LangChain now recommends building the supervisor directly from tools "for greater control over context engineering". [V-doc]

### 2.6 Cognee

Version 1.0 exposes `remember`, `recall`, `improve` and `forget` on top of `add`, `cognify`, `memify` and `search` ([README](https://github.com/topoteretes/cognee)). It keeps relational, vector and graph stores, and can run on a single Postgres as a demo. Without an LLM key it extracts with a local GLiNER model. Reported BEAM scores are 0.79 at 100k tokens and 0.67 at 10M (exploratory). [V-doc][Claim]

### 2.7 Research systems: A-MEM and MemoryOS

- **A-MEM** ([Xu et al., arXiv:2502.12110](https://arxiv.org/html/2502.12110), NeurIPS 2025). [V-paper]
  - Each note is `m = {content, timestamp, keywords, tags, context, embedding, links}`. Keywords, tags and context are LLM-generated. The embedding covers content, keywords, tags and context together.
  - Links: the top-k nearest notes by cosine are found first, then an LLM decides which links to keep.
  - "Evolution": the LLM rewrites the neighbours' context, keywords and tags.
  - Retrieval: top-k cosine.
  - Cost: about 1.2k–2.5k tokens per memory operation, against about 16.9k for the baselines.
- **MemoryOS** ([Kang et al., arXiv:2506.06326](https://arxiv.org/html/2506.06326)). [V-paper]
  - Short-term memory is a queue of 7 pages, feeding mid-term "segments", then a long-term personal memory.
  - Heat = N_visit + L_interaction + exp(−Δt/μ), with μ = 10⁷ s and all three weights set to 1. A segment moves to long-term memory when its heat exceeds 5.
  - Retrieval picks the top 5 segments (cosine plus keyword Jaccard), then the top-k pages inside them.
  - LoCoMo with gpt-4o-mini: average F1 36.23, 4.9 LLM calls and 3,874 tokens per response. MemGPT gets 29.13. MemoryOS's re-run of A-MEM gets 26.55 with **13.0 LLM calls**, and A-MEM's own multi-hop 45.85 becomes 33.23.
- **Lesson:** write paths that call an LLM many times are expensive and hard to reproduce. A heat score (visits, size, recency) is cheap and needs no LLM. Codly reuses it for decay (§6.6). [I]

### 2.8 How coding agents handle memory in production

**Claude Code** ([memory docs](https://code.claude.com/docs/en/memory)). [V-doc]

- *CLAUDE.md* files come from several scopes: managed policy, project (`./CLAUDE.md` or `./.claude/CLAUDE.md`), user (`~/.claude/CLAUDE.md`), local (`CLAUDE.local.md`) and path-scoped `.claude/rules/`.
  - `@path` imports nest up to 4 levels.
  - A subdirectory's file loads only when Claude works in that directory.
  - Files should stay under 200 lines.
  - The content arrives "as a user message after the system prompt, not as part of the system prompt itself".
  - `AGENTS.md` is read when no CLAUDE.md exists (v2.1.277 and later).
- *Auto memory*: Claude writes notes of four types (`user`, `feedback`, `project`, `reference`) as topic files with YAML frontmatter, and records a `modified` timestamp.
  - A `MEMORY.md` index sits beside them. Its first 200 lines or 25 KB load at the start of every session; topic files are read on demand.
  - It lives at `~/.claude/projects/<project>/memory/`. The path is "derived from the git repository, so all worktrees and subdirectories within the same repo share one auto memory directory."
  - Controls: `autoMemoryDirectory` (allowed in any settings scope, including `--settings`), `autoMemoryEnabled`, and `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`.
  - Claude "skips anything it can derive from the codebase".
  - Subagents don't inherit the main session's auto memory.
  - The stream's `system/init` event reports `memory_paths.auto`, and a `system/memory_recall` subtype exists (sibling observation, [cli-claude-codex.md](./cli-claude-codex.md)). Consequences for Codly are in §5.6. [V-local]

**Codex.** [V-doc]

- *AGENTS.md* ([docs](https://learn.chatgpt.com/docs/agent-configuration/agents-md)): first the global `~/.codex/AGENTS.override.md` or `AGENTS.md`, then one file per directory from the project root down to the working directory, joined root-first. Combined size is capped by `project_doc_max_bytes` (32 KiB by default). `project_doc_fallback_filenames` adds other names, and `CODEX_HOME` moves the home directory.
- *Memories* ([docs](https://learn.chatgpt.com/docs/customization/memories?surface=app)) are off by default and enabled with `[features] memories = true`.
  - After a thread goes idle, Codex turns useful context into local files under `~/.codex/memories/` in the background and redacts secrets.
  - Settings: `memories.generate_memories`, `memories.use_memories`, `memories.disable_on_external_context` (skip threads that used MCP or web search), `memories.min_rate_limit_remaining_percent`, `memories.extract_model` and `memories.consolidation_model`.
  - The docs advise: "Treat memories as a helpful recall layer, not as the only source for rules that must always apply."

**Gemini CLI** ([docs](https://geminicli.com/docs/tools/memory/)) edits markdown memory with `write_file` and `replace`, across three tiers: the repository's `GEMINI.md`, a private per-project folder, and the global `~/.gemini/GEMINI.md`. The same page says Gemini CLI "was replaced by Antigravity CLI on June 18, 2026", which the CLI reports should take into account. [V-doc]

**OpenCode** ([docs](https://opencode.ai/docs/rules/)) reads the project `AGENTS.md` and `~/.config/opencode/AGENTS.md`. It falls back to Claude Code's CLAUDE.md, unless `OPENCODE_DISABLE_CLAUDE_CODE*` is set, and accepts an `instructions` array of paths, globs or URLs. [V-doc]

**GitHub Copilot Memory** ([engineering blog, January 2026](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/); [docs](https://docs.github.com/en/copilot/concepts/agents/copilot-memory)) is the production design closest to Codly's needs.

- A memory is **subject, fact, citations (code locations), reason**, created through a `store_memory` tool during work.
- Before using a memory, the agent checks its citations against the current branch. If the code contradicts it, the agent stores a correction. If it holds, the agent re-stores it, which refreshes its timestamp.
- One memory pool per repository is shared by the coding agent, code review, the CLI and autofix.
- Docs: "any stored fact or preference that goes unused is automatically deleted after 28 days", and the timer resets when Copilot validates and uses an entry. [V-doc]
- Results [Claim, vendor A/B test]:
  - pull-request merge rate 83 % → 90 %;
  - code-review precision +3 % and recall +4 %;
  - positive feedback on review comments 75 % → 77 %;
  - deliberately planted false memories were caught and corrected.

### 2.9 Learning from experience: ReasoningBank and ACE

- **ReasoningBank** ([Ouyang et al., arXiv:2509.25140](https://arxiv.org/html/2509.25140), ICLR 2026). [V-paper]
  - Each item has a `title`, a one-line `description` and `content` (a strategy or pitfall).
  - An LLM judge labels every trajectory as a success or a failure, with 72.7 % agreement with ground truth. Accuracy stays stable for judges between 70 % and 90 % accurate. Each trajectory yields at most 3 items.
  - Retrieval puts the **top-1** item into the system prompt. Consolidation is a plain append.

  | Benchmark | Model | No memory | ReasoningBank |
  |---|---|---|---|
  | SWE-bench Verified, resolve rate | Gemini 2.5 Flash | 34.2 (30.3 steps) | 38.8 (27.5 steps) |
  | SWE-bench Verified, resolve rate | Gemini 2.5 Pro | 54.0 (21.1 steps) | 57.4 (19.8 steps) |

  On WebArena the gains are +4.6 to +8.3 points.
- **ACE** ([Zhang et al., arXiv:2510.04618](https://arxiv.org/html/2510.04618)). [V-paper]
  - Context is a "playbook" of itemized bullets. Each bullet has an id and counters for how often it was marked helpful or harmful.
  - A reflector distills lessons, and a curator emits **delta** items that "are merged into the existing context by deterministic, non-LLM logic". Duplicates are pruned by embedding similarity.
  - The headline warning: in one AppWorld run, a full LLM rewrite shrank the context from 18,282 to 122 tokens, and accuracy fell from 66.7 % to 57.1 %, below the 63.7 % of not adapting at all.
  - Gains: +10.6 % on agent tasks and +8.6 % on finance, with average adaptation latency down 86.9 %.
- **Lesson for Codly:** store "lessons" as small itemized records with counters. Merge them deterministically. Never let a model rewrite the whole set. That is MD's reflector with the risk removed. [I]

### 2.10 Anthropic's guidance

- **Context engineering** ([September 29, 2025](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)). [V-doc]
  - Context is "a finite resource with diminishing marginal returns", and longer contexts suffer "context rot".
  - Prefer just-in-time retrieval through "lightweight identifiers", combined with a small upfront load (CLAUDE.md, then glob and grep).
  - For long tasks: compaction; structured note-taking ("NOTES.md", to-do lists); and subagents that return a "condensed, distilled summary", often 1,000–2,000 tokens.
- **Memory tool** ([docs](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)). [V-doc]
  - `{"type":"memory_20250818","name":"memory"}` runs on the client. Claude asks for `view`, `create`, `str_replace`, `insert`, `delete` and `rename` under `/memories`, and the application executes them.
  - The API adds "ALWAYS VIEW YOUR MEMORY DIRECTORY BEFORE DOING ANYTHING ELSE … ASSUME INTERRUPTION" to the system prompt.
  - Security advice: block path traversal, cap file size, "periodically delete memory files that haven't been accessed in a long time", and strip sensitive data.
  - A multi-session recipe: an initializer session writes a progress log and a feature checklist, and every later session reads them first and updates them before ending.

  Codly can't add this tool to the CLIs, but the same semantics fit an MCP tool.
- **Multi-agent research system** ([June 13, 2025](https://www.anthropic.com/engineering/multi-agent-research-system)). [V-doc]
  - The lead agent saves its plan to memory, because "if the context window exceeds 200,000 tokens it will be truncated".
  - Subagents "store their work in external systems, then pass lightweight references back".
  - Each task description needs "an objective, an output format, guidance on the tools and sources to use, and clear task boundaries".
  - Multi-agent runs use about 15× the tokens of a chat, and token usage alone explains 80 % of the performance variance.
- **Claude Code agent teams** ([docs](https://code.claude.com/docs/en/agent-teams), experimental). [V-doc]
  - A lead, teammates, a shared task list and a per-agent JSON mailbox at `~/.claude/teams/{team}/inboxes/{agent}.json`.
  - Tasks are pending, in progress or completed, with dependencies, and claims use file locking.
  - Teammates load CLAUDE.md, MCP servers and skills, but "the lead's conversation history does not carry over".
  - A message from another agent never counts as the user's consent.
  - Recommended size: 3–5 teammates with 5–6 tasks each.
- **Hooks** ([docs](https://code.claude.com/docs/en/hooks)). [V-doc]
  - `hookSpecificOutput.additionalContext` (on SessionStart, UserPromptSubmit, PostToolUse and Stop) is capped at 10,000 characters per field.
  - "Phrase the text as factual statements, not imperative system commands, or Claude's prompt-injection defenses may surface it to you instead of using it."

### 2.11 Benchmarks: what they actually measure

- **LongMemEval** ([Wu et al., ICLR 2025](https://arxiv.org/abs/2410.10813); [repo](https://github.com/xiaowu0162/LongMemEval)). [V-paper][V-doc]
  - 500 questions testing information extraction, multi-session reasoning, temporal reasoning, knowledge updates and abstention.
  - LongMemEval_S has about 115k tokens (~40 sessions) of history per question; LongMemEval_M has about 500 sessions.
  - Answers are judged by GPT-4o, and a separate script reports retrieval recall at session and turn level.
  - The paper finds commercial assistants and long-context models lose about 30 % accuracy across sustained interactions.
- **LoCoMo** ([Maharana et al., 2024](https://arxiv.org/abs/2402.17753)): about 300 turns and 9k tokens per conversation, up to 35 sessions. [V-paper] Its scoring is disputed:
  - how to treat category 5 (Zep vs Mem0);
  - an audit claims 6.4 % of the answer key is wrong and that the judge accepts up to 63 % of intentionally wrong answers ([Penfield Labs](https://penfieldlabs.substack.com/p/we-audited-locomo-64-of-the-answer));
  - conversations now fit inside model context windows. [Claim]
- **BEAM** (1M and 10M tokens) is what Mem0 and Cognee now report for scale. [Claim]
- **No public benchmark measures coding-agent memory.** The closest evidence is Copilot's A/B test and ReasoningBank's SWE-bench numbers.
- **Retrieval recall is not answer accuracy.** MemPalace: 96.6 % R@5, about 67 % QA. [V-paper] Codly should report both (§6.10).

### 2.12 Security: memory is an injection channel

- **MINJA** ([arXiv:2503.03704](https://arxiv.org/abs/2503.03704), NeurIPS 2025) plants malicious records in an agent's memory through ordinary queries alone: 98.2 % injection success and 76.8 % attack success. [V-paper]
- In a multi-agent office, a web page one worker reads could become a "fact" every other worker sees.
- Mitigations used below [I]:
  - every memory carries provenance and a trust level;
  - anything derived from untrusted tool output is never promoted automatically;
  - packs present memories as quoted data with their source, never as instructions;
  - secrets are scanned out before writing;
  - project and global scopes go through review;
  - code facts are checked against their citations.

### 2.13 Synthesis: what actually works

1. **Keep the raw record and derive memory from it.** Events and reports stay immutable. Memories are derived, append-only and superseded rather than deleted. Mem0 2026, Zep's invalidation, MemPalace's verbatim store and ACE's deltas all point this way.
2. **Retrieval quality dominates.** Use hybrid lexical and dense search with scope filters and entity boosts. Mem0 2026, Graphiti and mempal (§3.6) all do this.
3. **Agents use simple, familiar tools well.** Letta's file tools, Claude and Codex's files, and Letta's git repos. Keep the tool count small (Anthropic).
4. **Keep a small always-on core, and fetch the rest just in time.** Examples: Claude's index of 200 lines or 25 KB, MemPalace's 600–900-token wake-up, Letta's `system/` folder.
5. **Code memories need citations and checks before use** (Copilot).
6. **Turn experience into lessons, from failures as well as successes**, as small itemized records with counters (ReasoningBank, ACE).
7. **Move LLM work off the hot path and batch it.** Examples: LangMem's background mode, Codex's background passes, Letta's sleep-time agents, MD's reflector.
8. **Measure honestly.** Report retrieval R@k *and* end-to-end task outcomes, and never compare vendor numbers across setups.

---

## 3. Triple extraction and the knowledge graph

### 3.1 What Codly's graph is for

The graph holds two kinds of facts. [I]

1. **Activity and provenance facts.** These are deterministic and come from events: agents, tasks, files, commands, commits, messages and memories. They answer questions like these:
   - Who last changed `src/auth/session.ts`, for which task, and why?
   - Which tasks touched the payments module this week?
   - What failed the last time someone ran the e2e suite?
   - Which decision explains why we use pnpm?
2. **Semantic facts.** These come from task reports, explicit writes, user pins or optional LLM extraction. Examples:
   - `project USES_PACKAGE_MANAGER pnpm`
   - `module:auth REQUIRES service:redis`
   - `user PREFERS "2-space indentation"`

**Code structure stays in the code index.** Definitions, imports and calls belong to the code index of [context-engineering.md](./context-engineering.md). Graph nodes for files and symbols point at code-index ids instead of copying that structure.

**Most semantic edges point at a memory** through `memory_rid`. The memory holds the readable text, the citations and the provenance. The graph is how agents navigate from one fact to related ones. It is not a second, competing source of truth.

### 3.2 Path A: rules over tool events (deterministic, free, always on)

The orchestration report already normalizes every CLI's stream into one event vocabulary ([agents-backend-orchestration.md](./agents-backend-orchestration.md) §3.6–3.7: `ToolStarted`/`ToolCompleted` with a kind of read, search, edit, execute and so on; task state changes; commits). Rules over those events give accurate triples with no LLM:

| Event | Triple(s) | Noise control |
|---|---|---|
| `ToolCompleted{kind: edit/create/delete, paths}` in run R (agent A, task T) | `A EDITED file:p`; `T TOUCHES file:p` | One edge per (A, p, T), upserted. `weight` counts edits; keep the first and last timestamps. |
| `ToolCompleted{kind: execute, command, exit}` | `T RAN_OK cmd:c` or `T RAN_FAILED cmd:c`, with exit code and duration in the edge's props | Keep the latest N per task. |
| `ToolCompleted{kind: read/search}` | none | Too noisy. Count reads in entity props instead. |
| Task assigned, started or finished | `A WORKED_ON T`; `T DEPENDS_ON T2` (copied from `task_dep`) | |
| Commit or merge | `commit:sha IMPLEMENTS T`; `T MERGED_INTO branch:b` | |
| `task_report.decisions[]` (§5.3) | `T DECIDED mem:m`; `mem:m ABOUT <entity>` | |
| `memory_write` with citations | `mem:m ABOUT file:p` | |
| Scope lease (orchestration report §6.2) | `T OWNS_SCOPE glob:g` while held | Closed when released. |
| Message with `act = blocker` | `T BLOCKED_BY <entity or agent>` | Closed when the task unblocks. |

A rough scale: about 20–60 edges per task, so a thousand tasks give tens of thousands of edges. That is small for SQLite (§3.7). [I]

```rust
// Sketch, not compiled. Event and type names are illustrative and follow the
// normalized AgentEvent enum of agents-backend-orchestration.md §3.6.
pub fn rule_edges(ev: &AgentEvent, ctx: &RunCtx) -> Vec<EdgeUpsert> {
    let agent = EntityKey::agent(&ctx.agent_id);
    let task = ctx.task_id.as_deref().map(EntityKey::task);
    let mut out = Vec::new();
    match ev {
        AgentEvent::ToolCompleted { kind: ToolKind::Edit | ToolKind::Create | ToolKind::Delete,
                                    paths, ok: true, .. } => {
            for p in paths {
                // every worker runs in its own worktree: key files by repo-relative path
                let file = EntityKey::file(&ctx.repo_relative(p));
                out.push(EdgeUpsert::aggregate(agent.clone(), "EDITED", file.clone(), ev.src()));
                if let Some(t) = &task {
                    out.push(EdgeUpsert::aggregate(t.clone(), "TOUCHES", file, ev.src()));
                }
            }
        }
        AgentEvent::ToolCompleted { kind: ToolKind::Execute, command, exit_code, .. } => {
            if let Some(t) = &task {
                let cmd = EntityKey::command(&normalize_cmd(command)); // strip env, collapse paths
                let pred = if *exit_code == Some(0) { "RAN_OK" } else { "RAN_FAILED" };
                out.push(EdgeUpsert::latest(t.clone(), pred, cmd, ev.src()));
            }
        }
        _ => {}
    }
    out
}
```

### 3.3 Path B: LLM extraction through the user's own CLI (opt-in, batched)

**When.** At the end of a task, batched (several tasks per call, or when the app is idle), and only if the user has turned it on. Skip the pass when rate-limit headroom is low; Codex does the same with `memories.min_rate_limit_remaining_percent`. [I]

**Input.** Not the transcript. Send:
- the worker's structured `task_report` (§5.3);
- the diff stat and the list of changed files;
- the user's original request;
- at most a few key messages.

That keeps the input to roughly 2k–6k tokens. [I]

**Command.** These flags were verified by the sibling probe in [agents-backend-orchestration.md](./agents-backend-orchestration.md) §5.3. That call took about 3.2 s and wrote about 1.5k cache tokens with a custom system prompt (4.6k with the default prompt), and `haiku` resolved to `claude-haiku-5-5`. [V-local, sibling]

```bash
cd "$(mktemp -d)"   # empty cwd: -p loads project hooks and .mcp.json without a trust prompt
CLAUDE_CODE_DISABLE_AUTO_MEMORY=1 claude -p --model haiku --effort low \
  --output-format json --no-session-persistence \
  --tools "" --setting-sources "" --strict-mcp-config \
  --system-prompt-file "$CODLY_RES/prompts/extract-v1.txt" \
  --json-schema "$(cat "$CODLY_RES/schemas/extract-v1.json")" < task-bundle.md
# Use .structured_output. If the field is missing (refusal or failure), keep nothing.
```

- **Don't use `--bare`.** It "never reads OAuth credentials" and needs `ANTHROPIC_API_KEY`, so it would not run on the user's subscription ([headless docs](https://code.claude.com/docs/en/headless)). [V-doc]
- **Billing.** Anthropic paused the separate Agent SDK credit pool. `claude -p` usage "still draw[s] from your subscription limits" (help page updated 2026-10-07). [V-doc]
- **Codex alternative.** `codex exec --output-schema <file>`; the flag is present in Codex's `exec` source. [V-src]

**Schema** (condensed; the full version caps every string):

```json
{"type":"object","additionalProperties":false,"required":["memories","triples"],
 "properties":{
  "memories":{"type":"array","maxItems":5,"items":{"type":"object","additionalProperties":false,
    "required":["kind","scope","title","body","confidence","evidence"],
    "properties":{
      "kind":{"enum":["fact","decision","preference","lesson"]},
      "scope":{"enum":["task","project","agent"]},
      "title":{"type":"string","maxLength":120},
      "body":{"type":"string","maxLength":800},
      "citations":{"type":"array","maxItems":5,"items":{"type":"object","additionalProperties":false,
        "required":["path"],"properties":{"path":{"type":"string"},"start":{"type":"integer"},"end":{"type":"integer"}}}},
      "entities":{"type":"array","maxItems":8,"items":{"type":"string"}},
      "confidence":{"type":"number","minimum":0,"maximum":1},
      "evidence":{"type":"string","maxLength":280}}}},
  "triples":{"type":"array","maxItems":10,"items":{"type":"object","additionalProperties":false,
    "required":["subject","subject_type","predicate","object","object_type","confidence","evidence"],
    "properties":{
      "subject":{"type":"string"},
      "subject_type":{"enum":["file","module","package","service","command","concept","person"]},
      "predicate":{"type":"string","pattern":"^[A-Z][A-Z0-9_]{2,40}$"},
      "object":{"type":"string"},
      "object_type":{"enum":["file","module","package","service","command","concept","person","value"]},
      "valid_from":{"type":["string","null"]},
      "confidence":{"type":"number","minimum":0,"maximum":1},
      "evidence":{"type":"string","maxLength":280}}}}}}
```

**Prompt rules**, borrowed from Graphiti's edge prompt (§2.3) and ReasoningBank's limits (§2.9): [I]
- Use predicates from Codly's registry, and propose a new one only when nothing fits.
- Keep concrete specifics.
- Never invent dates.
- Lessons cover failures as well as successes.
- At most 5 memories and 10 triples per task.
- Every item quotes its evidence.

**Validation in Rust** before anything is stored: [I]
1. Check the schema and the type constraints.
2. Check that each cited path exists in the task's head commit, and that each line range is within the file.
3. Resolve entities (§3.4).
4. Deduplicate against existing memories and edges.
5. Store items with `confidence < 0.6`, and anything whose evidence comes from untrusted tool output, as `proposed`.
6. Store the rest as `active`, at task or agent scope only. Writes to project scope go to review unless the user turns on auto-accept.

**Why not Graphiti-style ingestion.** Graphiti runs entity extraction with reflection, entity resolution, edge extraction, deduplication, time extraction and invalidation for *every episode* (§2.3). That suits a cloud service. On a subscription, with many agents taking turns, it would be expensive. Microsoft's LazyGraphRAG shows the cheap alternative: extract noun phrases with plain NLP at index time ("indexing costs are identical to vector RAG and 0.1% of the costs of full GraphRAG") and spend LLM calls only at query time ([Microsoft Research](https://www.microsoft.com/en-us/research/blog/lazygraphrag-setting-a-new-standard-for-quality-and-cost/)). [V-doc] Codly's split works the same way: rules and worker-written reports at write time, and on-demand reasoning by the agent that is already running.

### 3.4 Entity resolution

1. **Typed entities get deterministic keys.** No fuzzy matching is needed for these. [I]

   | Type | Key |
   |---|---|
   | file | repo-relative POSIX path, with the worktree root stripped (each worker has a different absolute path) |
   | symbol | the code index id |
   | package | `npm:react`, `cargo:serde` |
   | command | normalized argv: environment assignments removed, absolute paths collapsed |
   | agent, task, plan, commit | their ids |

2. **Concepts, services and people** go through these steps: [I]
   1. Normalize: NFKC, lowercase, collapse whitespace, strip punctuation.
   2. Look up the exact alias.
   3. Fuzzy match. Skip it, as Graphiti does, when the name is under 6 characters with fewer than 2 tokens, or has entropy below 1.5. Otherwise get candidates from an FTS5 `trigram` index over entity names, then accept the best candidate of the same type with 3-gram Jaccard ≥ 0.9 (Graphiti's threshold, §2.3). At Codly's scale the trigram index replaces MinHash LSH.
   4. Otherwise accept a name embedding with cosine ≥ 0.92 within the same type.
   5. Otherwise create a new entity and flag it for a batched merge review by an LLM or the user.
3. **Merges are reversible.** A merged entity gets `merged_into`, and its aliases are kept.

### 3.5 Temporal model

- **Two time axes, as in Graphiti.** `valid_from` and `valid_to` say when a fact held in the world. `recorded_at` and `expired_at` say when Codly learned it and when Codly withdrew it. Intervals are half-open, `[from, to)`. [V-paper Zep][I]
  - A fact that **stopped being true** gets `valid_to`.
  - A fact that **was never true**, such as an extraction error the user rejected, gets `expired_at`.

  This lets the UI answer both "what was true on October 3?" and "what did we believe on October 3?".
- **Supersede at one shared instant.** Close the old fact and open the new one with the same timestamp, so an as-of query at the boundary returns only the successor. MemPalace's `supersede()` documents why: when an `invalidate` and an `add` both use a date-only value, they overlap for that whole day. [V-src]
- **Functional predicates supersede automatically.** A predicate flagged in the registry allows only one current object per subject, so a new value closes the old one. Examples: `USES_PACKAGE_MANAGER`, `TEST_COMMAND`, `DEFAULT_BRANCH`, `OWNED_BY`. Non-functional predicates, such as `REQUIRES`, may have several current objects. A conflict that is only semantic (similar text, opposite claims) is never resolved automatically: it goes to review (§6.6). [I]
- **Code facts** are checked through their citations (§6.6) rather than tied to commit ranges. A failed check sets the memory to `stale`; it neither deletes the memory nor closes its validity. [I]

```sql
-- As-of query: what was true at :t, as known at :k
SELECT s.name AS subject, e.predicate, COALESCE(o.name, e.object_value) AS object,
       e.valid_from, e.valid_to, e.memory_rid
FROM kg_edge e
JOIN kg_entity s ON s.id = e.subject_id
LEFT JOIN kg_entity o ON o.id = e.object_id
WHERE e.subject_id = :s
  AND e.valid_from <= :t AND (e.valid_to IS NULL OR e.valid_to > :t)
  AND e.recorded_at <= :k AND (e.expired_at IS NULL OR e.expired_at > :k);
```

```rust
// Supersede a functional fact in one transaction at one instant (sketch, rusqlite 0.41, not compiled).
fn supersede(tx: &rusqlite::Transaction, ws: &str, s: i64, p: &str,
             new: ObjRef, at: i64, src: &ExtractSrc) -> rusqlite::Result<i64> {
    let closed: Vec<i64> = tx.prepare(
        "UPDATE kg_edge SET valid_to = ?1
          WHERE subject_id = ?2 AND predicate = ?3 AND valid_to IS NULL AND expired_at IS NULL
          RETURNING id")?
        .query_map(rusqlite::params![at, s, p], |r| r.get(0))?.collect::<Result<_, _>>()?;
    tx.execute(
        "INSERT INTO kg_edge(workspace_id, subject_id, predicate, object_id, object_value,
                             valid_from, recorded_at, confidence, extractor, memory_rid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8, ?9)",
        rusqlite::params![ws, s, p, new.id(), new.value(), at, src.confidence, src.extractor, src.memory_rid])?;
    let id = tx.last_insert_rowid();
    for c in closed {
        tx.execute("UPDATE kg_edge SET superseded_by = ?1 WHERE id = ?2", rusqlite::params![id, c])?;
    }
    Ok(id)
}
```

### 3.6 SQLite versus an embedded graph database

| Option | Crate and version (crates.io today) | License | Status | Fit for Codly |
|---|---|---|---|---|
| **SQLite tables + recursive CTEs** | `rusqlite` 0.41.0 (2026-10-24), bundled SQLite 3.54.1 with FTS5 compiled in | MIT; SQLite is public domain | very active | **Best fit.** One file, one backup, and transactions shared with memories. Traversal takes milliseconds at Codly's scale (§3.7). [V-reg][V-local] |
| Kuzu (embedded Cypher) | `kuzu` 0.11.3 (2025-10-10, the final release) | MIT | archived 10 Oct 2025; Graphiti deprecated it ([The Register](https://www.theregister.com/2025/10/14/kuzudb_abandoned/)) | No. [V-reg][V-doc] |
| LadybugDB (Kuzu fork) | `lbug` 0.22.0 (2026-10-25) | MIT | active, young fork | Not now. It adds a C++ engine and a second store. Revisit only for large analytical graphs. [V-reg] |
| Oxigraph (RDF/SPARQL) | `oxigraph` 0.5.13 (2026-10-22) | MIT OR Apache-2.0 | active | Overkill. Validity windows need reification or named graphs. [V-reg] |
| CozoDB (Datalog) | `cozo` 0.7.6 (2023-12-11) | MPL-2.0 | no release since 2023 | No. [V-reg] |
| SurrealDB (embedded) | `surrealdb` 3.3.4 | BSL 1.1, "not an Open Source license" | active | No: licence and size. [V-reg][V-doc] |
| HelixDB | `helix-db` 3.0.0 (an SDK crate) | Apache-2.0 | active, runs as a server | No: it needs a separate server. [V-reg] |
| petgraph (in memory) | `petgraph` 0.8.3 | MIT OR Apache-2.0 | stable | **As a derived view only**: PageRank, centrality and communities for the graph panel. Not as storage. [V-reg] |

**Prior art in Rust.** `mempal` 0.9.1 (MIT) is a "project memory tool for coding agents": a single SQLite file with FTS5 and sqlite-vec fused by RRF, a model2vec embedder, triples with `valid_from`/`valid_to` and a fact checker, and about 10 MCP tools ([docs.rs](https://docs.rs/crate/mempal/0.9.1)). It closely matches the design proposed here, but adoption is tiny (137 recent downloads). Read it for ideas; don't depend on it. [V-reg][V-doc]

### 3.7 Traversal SQL and local measurements

```sql
-- k-hop neighbourhood. Two recursive arms, one per index; allowed since SQLite 3.34.0.
WITH RECURSIVE walk(eid, depth) AS (
  SELECT :seed, 0
  UNION                                            -- UNION, not UNION ALL: stops on cycles
  SELECT e.object_id, w.depth + 1 FROM walk w
    JOIN kg_edge e ON e.subject_id = w.eid
   WHERE w.depth < :max_depth AND e.valid_to IS NULL AND e.expired_at IS NULL
  UNION
  SELECT e.subject_id, w.depth + 1 FROM walk w
    JOIN kg_edge e ON e.object_id = w.eid
   WHERE w.depth < :max_depth AND e.valid_to IS NULL AND e.expired_at IS NULL
  LIMIT 5000                                       -- safety cap, as the SQLite docs advise
)
SELECT eid, MIN(depth) AS depth FROM walk GROUP BY eid;
```

The SQLite documentation confirms both points: multiple recursive SELECTs "added in version 3.34.0", and UNION being used "to prevent the recursion from entering an infinite loop if the graph contains cycles" ([lang_with](https://www.sqlite.org/lang_with.html)). [V-doc]

**Measurements.** [V-local; scripts: `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/sqlbench.py`, `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/sqlbench2.py`]

Setup: 50k entities, 250k edges (80 % current), skewed degree distribution, in-memory database, Python `sqlite3` with SQLite 3.51.0, the same 60 seeds for every formulation, partial indexes `(subject_id, predicate)` and `(object_id, predicate)` where the edge is current.

| Query | Formulation | p50 | p95 | Median nodes reached |
|---|---|---|---|---|
| 2 hops | **two-arm** (above) | **0.15 ms** | 1.00 ms | 97 |
| 2 hops | single arm with `ON (subject_id = eid OR object_id = eid)` | 82.7 ms | 245 ms | 97 |
| 3 hops | **two-arm** | **3.5 ms** | 23.7 ms | 2,295 |
| 3 hops | OR-join | 1,222 ms | 8,935 ms | 2,295 |
| 2 hops | `UNION ALL` subquery of both directions | 227 ms | 229 ms | (materializes all 250k edges) |
| As-of facts for one entity | indexed point query | 0.004 ms | 0.009 ms | |

**Gotcha.** The OR-join is only fast (0.14 ms at 2 hops in a first run) when *non-partial* indexes on `subject_id` and `object_id` also exist, which lets the planner use its multi-index OR strategy. With only partial indexes it is roughly 350 to 550 times slower. The two-arm form returned identical results and is fast with either kind of index. Always write traversals that way.

**Recommendation.**
- Store the graph in SQLite next to memories.
- Write traversals with two arms, at most 2 hops for agents and the UI, and always with a `LIMIT`.
- Build an in-memory `petgraph` only for analytics in the graph panel.
- Reconsider a graph engine only if the graph passes about 10M edges or needs real graph analytics in SQL. [I]

---

## 4. Local embeddings and retrieval in Rust

### 4.1 Models

Memory entries are short natural-language sentences: facts, decisions, lessons. A small general English model fits. Code retrieval is the sibling report's job; it is lexical-first, with no embeddings in v1.

| Model | Params | Dim | Max tokens | License | Retrieval score (source) | In fastembed 7.5 |
|---|---|---|---|---|---|---|
| all-MiniLM-L6-v2 (MD's default) | 22.7M | 384 | 256 (trained at 128) | Apache-2.0 | MTEB retrieval NDCG@10 41.95 ([arctic card](https://huggingface.co/Snowflake/snowflake-arctic-embed-xs)) | yes |
| **bge-small-en-v1.5** | 33.4M | 384 | 512 | MIT | 51.68; MTEB average 62.17 ([card](https://huggingface.co/BAAI/bge-small-en-v1.5)) | yes (the default, plus a `Q` quantized variant) |
| snowflake-arctic-embed-xs | 22M | 384 | 512 | Apache-2.0 | 50.15 ([card](https://huggingface.co/Snowflake/snowflake-arctic-embed-xs)) | yes |
| EmbeddingGemma-300M (MD's alternative) | 0.3B | 768 (Matryoshka 512, 256, 128) | 2,048 | Gemma terms (gated download) | MTEB English v2 69.67, multilingual 61.15, code 68.76 (mean per task; [card](https://huggingface.co/google/embeddinggemma-300m)) | yes (`EmbeddingGemma300M`, Q4 variant) |
| Qwen3-Embedding-0.6B | 0.6B | up to 1024 (32–1024) | 32k | Apache-2.0 | MTEB multilingual 64.33, English v2 70.70 ([card](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B)) | yes (`qwen3` feature, candle backend) |
| potion-base-8M (static) | 7.5M | — | — | MIT | retrieval 31.11, against MiniLM's 42.92 in the same table ([model2vec results](https://github.com/MinishLab/model2vec/blob/main/results/README.md)) | no (model2vec-rs) |
| potion-retrieval-32M (static) | 32.3M | — | — | MIT | retrieval 35.06 (same source) | no |
| potion-code-16M-v2 (static, code) | 16.2M | 256 | — | MIT | CoIR NDCG@10 39.08, against **BM25 at 42.31**; hybrid 43.36; its teacher CodeRankEmbed 59.14 ([card](https://huggingface.co/minishlab/potion-code-16M-v2)) | no |
| jina-embeddings-v2-base-code | 161M | 768 | 8,192 | Apache-2.0 | no scores on the card ([card](https://huggingface.co/jinaai/jina-embeddings-v2-base-code)) | yes |
| CodeRankEmbed | 137M | 768 | 8,192 | MIT | CoIR 60.1; CodeSearchNet MRR 77.9 ([card](https://huggingface.co/nomic-ai/CodeRankEmbed)) | no (needs an ONNX export) |

Scores come from different MTEB snapshots: MiniLM's retrieval score is 41.95 in one table and 42.92 in the other. Compare rows only within one source. [V-doc]

### 4.2 Runtimes and latency on Apple Silicon

Published, reproducible benchmark ([jerrythomas/rust-embedding-bench](https://github.com/jerrythomas/rust-embedding-bench)): Apple M4 Max, macOS 26.3.1, all-MiniLM-L6-v2. "eps" means embeddings per second. [Claim, third-party]

| Backend | Batch 1, short text, p50 | Batch 32, short text | Batch 32, long text |
|---|---|---|---|
| llama-cpp-2 (Metal, GGUF) | 1.26 ms | 9,676 eps | 948 eps |
| `ort` fp32, 8 threads | **1.10 ms** | 3,052 eps | 261 eps |
| `fastembed` | **1.71 ms** | 2,849 eps | 315 eps |
| `candle`, 8 threads (CPU) | 8.15 ms | 603 eps | 31 eps |
| Ollama over HTTP | 11.05 ms | 433 eps | 223 eps |

- "Dynamic int8 quantization was a wash on Apple Silicon": no faster than fp32, and cosine similarity to the reference fell to 0.961. Every other backend stayed at 0.999 or better.
- Embedding a memory as it is written therefore costs a few milliseconds. bge-small has 12 transformer layers to MiniLM's 6, at the same 384 width ([config](https://huggingface.co/BAAI/bge-small-en-v1.5/blob/main/config.json)), so expect roughly twice MiniLM's time per text, about 2–4 ms [I]. snowflake-arctic-embed-xs is "based on the all-MiniLM-L6-v2 model", so it costs the same as MiniLM while scoring 50.15 instead of 41.95 [V-doc]. Re-embedding 10k memories in the background takes seconds.
- **Don't use the CoreML execution provider for embeddings.** MD saw NaN vectors from CoreML with quantized EmbeddingGemma, and chromadb disables CoreML for MiniLM because it is "not as well optimized as CPU" (both from MD's code comments). [V-src]
- **model2vec-rs** claims about 8,000 samples/s on a single CPU thread ([README](https://github.com/MinishLab/model2vec-rs)) [Claim]. That suits an instant fallback or bulk work, but its retrieval quality is clearly lower (31–35 vs 42–52).

### 4.3 Crates (versions read from crates.io today)

| Crate | Version (date) | License | Notes |
|---|---|---|---|
| `fastembed` | 7.5.0 (2026-10-23) | Apache-2.0 | Pins `ort = "=2.0.0-rc.15"`; needs `tokenizers ^0.23.2` and `hf-hub ^0.5.0`. The default features add `image-models`, an ORT binary download at build time with native-tls, and `hf-hub`. For Codly use `default-features = false` plus `ort-download-binaries-rustls-tls` (or `ort-load-dynamic` with a bundled dylib), and ship the model files inside the app. ([features](https://docs.rs/crate/fastembed/7.5.0/features)) [V-reg][V-doc] |
| `ort` | 2.0.0-rc.15 (2026-10-19); no stable 2.0 yet | MIT OR Apache-2.0 | Pin exactly. [V-reg] |
| `candle-core`, `candle-nn`, `candle-transformers` | 0.11.0 (2026-06-26) | MIT OR Apache-2.0 | Pure Rust with Metal support; slower than ort for small BERT models on CPU (§4.2). [V-reg] |
| `model2vec-rs` | 0.3.0 (2026-09-19) | MIT per the repository (crates.io shows a "non-standard" license field) | Static embeddings. [V-reg][V-doc] |
| `tokenizers` | 0.23.2 stable; 1.0.0-rc.3 also published | Apache-2.0 | [V-reg] |
| `rusqlite` | 0.41.0 (2026-10-24) | MIT | `bundled` compiles FTS3/FTS5, JSON1, RTREE and LOAD_EXTENSION unconditionally ([build.rs](https://raw.githubusercontent.com/rusqlite/rusqlite/master/libsqlite3-sys/build.rs)). [V-reg][V-src] |
| `sqlite-vec` | 0.1.9 stable (2026-03-31); 0.1.10-alpha.4 (2026-05-18, docs.rs build failed) | MIT/Apache-2.0 | The stable release has brute-force `vec0`. ANN indexes (rescore, IVF, DiskANN) exist only in the 0.1.10 alphas ([releases](https://github.com/asg017/sqlite-vec/releases)). [V-reg][V-doc] |
| `usearch` | 2.26.6 (2026-10-25) | Apache-2.0 | HNSW with a C++ core. [V-reg] |
| `hnsw_rs` | 0.3.5 (2026-10-09) | MIT/Apache-2.0 | Pure-Rust HNSW. [V-reg] |
| `arroy` | 0.9.0 (2026-10-22) | MIT | Meilisearch's ANN on LMDB. [V-reg] |
| `lancedb` | 0.40.0 (2026-10-07) | Apache-2.0 | Columnar vector database; heavy. [V-reg] |
| `instant-distance` | 0.6.1 (2023-06-26) | MIT OR Apache-2.0 | No release since 2023. [V-reg] |
| `simsimd` | 6.5.16 (2026-03-07) | Apache-2.0 | SIMD distances, if hand-written loops ever fall short. [V-reg] |
| `tantivy` | 0.27.0 (2026-10-17) | MIT | Not needed while FTS5 is enough. [V-reg] |
| `rmcp` | 3.7.0 (2026-10-22) | Apache-2.0 | Implements MCP `2026-07-28` and stays compatible with `2025-11-25` ([repo](https://github.com/modelcontextprotocol/rust-sdk)). [V-reg][V-doc] |
| `petgraph` | 0.8.3 | MIT OR Apache-2.0 | [V-reg] |
| `tiktoken-rs` | 0.12.1 | MIT | Token *estimates* for budgeting. Claude's tokenizer isn't public, so leave a margin. [V-reg] |

### 4.4 Vector storage

**Measured brute force.** [V-local; script: `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/vecbench/`]

Setup: std-only Rust 1.96 with `lto = "fat"`, Apple M5 Pro, random L2-normalized 384-d vectors, top-20 results, 60 queries. Multi-threaded runs spawn their threads per query, so they include spawn cost. The p95 spikes reflect a busy machine; p50 is the stable signal.

| N | f32, 1 thread (p50 / p95) | f32, 8 threads | int8, 1 thread | f32 RAM |
|---|---|---|---|---|
| 10k | 0.45 / 0.57 ms | 0.24 / 0.30 ms | 0.23 / 0.24 ms | 15 MB |
| 50k | 2.34 / 13.4 ms | 0.99 / 1.08 ms | 1.32 / 11.2 ms | 77 MB |
| 100k | **4.85** / 11.4 ms | 2.94 / 4.67 ms | **2.42** / 2.75 ms | 154 MB |
| 250k | 11.4 / 14.5 ms | 3.59 / 4.26 ms | 4.08 / 6.92 ms | 384 MB |

**Decision.** [I]
- Store vectors as BLOBs (little-endian f32, normalized) in a plain table, with the model id and dimension next to each row.
- Load one contiguous matrix per open workspace, plus a scope-to-row map.
- Brute-force it on a persistent thread pool.
- Codly will hold somewhere between 10³ and 10⁵ memories, so brute force covers every case with room to spare, and needs no extension loading and no index to keep in sync.
- Revisit `sqlite-vec` once its ANN indexes reach a stable release, or `usearch` beyond about 250k vectors per workspace.

### 4.5 Hybrid retrieval: FTS5 + vectors + RRF

**Measured FTS5.** 100k synthetic memories with `porter unicode61`: a three-term OR query for the top 20 by `bm25()` takes p50 0.65 ms, p95 2.22 ms. With 10k memories, p50 is 0.10 ms. Bulk indexing runs at about 190k documents/s. The code-retrieval sibling measured 1.5 ms p50 over 536k code chunks. [V-local]

- **Sanitize queries.** Agent or user text must not reach FTS5 raw: `-`, `:`, `*` and quotes are FTS5 syntax. Turn it into quoted tokens joined with OR, and keep a `tok*` prefix form for identifiers. Weight the title column with `bm25(memory_fts, 2.0, 1.0, 0.5)`. [I]
- **Fusion.** Reciprocal Rank Fusion with k = 60, as in Alex Garcia's [SQLite hybrid-search write-up](https://alexgarcia.xyz/blog/2024/sqlite-vec-hybrid-search/index.html), Graphiti and mempal. Codly adds a **third ranked list: entity overlap**, meaning memories `ABOUT` the files and symbols in the current task's write scope or in the current turn. Mem0's 2026 algorithm gets its gains from exactly this kind of entity signal (§2.2). [V-doc][I]

```rust
// RRF over several ranked id lists (memory rids), then business re-scoring (§6.4). Sketch.
fn rrf(lists: &[Vec<i64>], k: f32) -> Vec<(i64, f32)> {
    let mut acc: std::collections::HashMap<i64, f32> = Default::default();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *acc.entry(*id).or_default() += 1.0 / (k + rank as f32 + 1.0);
        }
    }
    let mut v: Vec<_> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

// lexical: FTS5 top 50 by bm25; dense: brute-force cosine top 50; entities: memories linked
// ABOUT the task's entities, newest first. All lists are pre-filtered to readable scopes.
let fused = rrf(&[lexical_ids, dense_ids, entity_ids], 60.0);
```

### 4.6 Recommendation

1. Use **fastembed 7.5.0 with bge-small-en-v1.5** (fp32 ONNX) bundled in the app. snowflake-arctic-embed-xs is the lighter option: the same cost as MD's MiniLM, and only 1.5 points behind bge-small on retrieval. Offer EmbeddingGemma-300M or Qwen3-Embedding-0.6B as an opt-in "multilingual / higher quality" model; MD offers EmbeddingGemma for the same reason.
2. Record the model id per vector, and re-embed in the background when the model changes. Never mix models in one search; MemPalace has explicit guards for this. [V-src]
3. **FTS5 is always on, and vectors are optional.** If the model is missing, retrieval degrades to BM25 plus entity matching.
4. Brute-force cosine in Rust. No ANN index in v1.
5. Fuse with RRF, then re-score with Codly's rules (§6.4).

---

## 5. Supervisor context and communication between agents

### 5.1 What the evidence says

- **Anthropic's research system.** The lead agent persists its plan. Subagents "store their work in external systems, then pass lightweight references back". Every task description carries an objective, an output format, tool guidance and boundaries. Running subagents synchronously creates bottlenecks: the lead "can't steer subagents" and subagents "can't coordinate". [V-doc]
- **Cognition, ["Don't Build Multi-Agents"](https://cognition.com/blog/dont-build-multi-agents)** (June 12, 2025). "Share context, and share full agent traces, not just individual messages", and "actions carry implicit decisions". In their Flappy Bird example, two subagents made clashing style assumptions. [V-doc]
- **Claude Code agent teams.** A shared task list with dependencies and file-locked claims, a JSON mailbox per agent, and a spawn prompt as the only context a teammate gets ("the lead's conversation history does not carry over"). Advice: avoid file conflicts by assigning ownership, and start with 3–5 teammates. [V-doc]
- **Blackboard systems.** [V-paper]
  - In [Salemi et al. (arXiv:2510.01285)](https://arxiv.org/abs/2510.01285v2), a central agent posts requests on a shared board and specialist agents volunteer. This gave 13–57 % relative gains in end-to-end success over the best baseline.
  - [Han & Zhang (arXiv:2507.01701)](https://arxiv.org/html/2507.01701v1) add a control unit that picks which agents act based on the board's contents. They report competitive results with fewer tokens.
  - In [PatchBoard (arXiv:2605.29313)](https://arxiv.org/abs/2605.29313), 2026, agents propose JSON-Patch changes to a structured shared state, and a deterministic kernel checks each one against a schema and per-role write contracts. On ALFWorld it reached 84.6 % success with 45.5k tokens per success, against 61.6 % and 64.2k for Flock, and 30.8 % and 368.3k for LangGraph.
- **MetaGPT** ([arXiv:2308.00352](https://arxiv.org/abs/2308.00352)) keeps a "global message pool" with publish–subscribe by role, and structured documents instead of free dialogue. [V-paper]
- **Codly's own orchestration report** decided "Blackboard wins for v1": a Codly-owned SQLite board is the single source of truth, and agents don't message each other directly in v1 ([agents-backend-orchestration.md](./agents-backend-orchestration.md) §4.5). [I, sibling]

**Synthesis.** [I]
1. Shared, structured, *validated* state beats free-form chatter.
2. Workers need the **decisions** that constrain them, not their siblings' full traces, which are too expensive with parallel workers. Share decisions, interface contracts and assumptions through the board, targeted at the workers whose entities they touch.
3. The supervisor sees summaries and references, and pulls detail when it needs it.
4. Messages are for questions, answers and blockers, delivered at turn boundaries.

### 5.2 Codly's policy: isolate by default, share by subscription [I]

A worker's context is its contract, plus its pack, plus its **subscriptions**. Subscriptions are computed, not declared. A worker is subscribed to:

1. its own task;
2. its plan's decisions;
3. the entities in its read and write scope (files, modules, packages);
4. its DMs and its 1:1 channel with the user.

When a new decision memory, board change or message overlaps a worker's subscriptions, it reaches that worker as a delta at its next turn. This is MetaGPT-style publish–subscribe, keyed on entities instead of roles.

- **Assumptions become proposed decisions.** The contract asks workers to post any assumption that affects others (an interface, a name, a shared file) as a *proposed* `decision`. The supervisor approves or rejects it, and approved decisions go to every subscriber. This is Codly's answer to Cognition's "implicit decisions".
- **Transcripts are never shared automatically.** The supervisor can ask for a digest of a worker's last turns through a tool. The user can see everything.

### 5.3 Worker → supervisor: `task_report`

The contract requires a `codly_task_report` call as the worker's last step, and the server validates it.

```json
{
  "task_id": "T-12",
  "outcome": "done | partial | blocked | failed",
  "summary": "≤ 600 chars: what changed and why",
  "decisions": [{"title": "Session cookies are SameSite=Lax", "body": "…", "scope": "task|project",
                 "citations": [{"path": "src/auth/cookie.ts", "start": 12, "end": 30}]}],
  "lessons":   [{"title": "Auth tests need Redis", "body": "Run docker compose up redis first; otherwise the suite times out after 60 s.",
                 "polarity": "failure"}],
  "files_changed": ["src/auth/cookie.ts", "tests/auth/cookie.test.ts"],
  "verification":  [{"command": "pnpm -F web test auth", "result": "pass"}],
  "open_questions": ["Should logout also revoke refresh tokens?"],
  "follow_ups":     [{"title": "Rotate the signing key script", "body": "…"}]
}
```

- **Ingestion.** [I]
  - Decisions become `decision` memories: active at task scope, proposed at project scope.
  - Lessons become `lesson` memories that keep their polarity, failures included, as in ReasoningBank.
  - Follow-ups become cards in the shared board's backlog.
  - Open questions go to the supervisor, or to the user's "Ask me" view.
- **Fallback.** If a CLI has no MCP access, or the worker didn't call the tool, Codly asks one extra turn for the same JSON. It uses `--json-schema` for Claude or `--output-schema` for Codex where available, and otherwise parses the final message. [I]
- **Cost.** No extra LLM call: the worker writes the report inside its own run. This is MD's "at the END of a task, append what you learned", but structured.

### 5.4 Supervisor → worker: contract and pack

**The contract.** The sibling's `task` row already carries the goal, acceptance criteria, read and write scopes and verification commands. Codly adds `context_refs`: memory, card and message ids the supervisor attaches as *references*. The broker expands them within the budget.

**Worker spawn pack.** At most 1,200 tokens for memory, next to the sibling's code-pointer pack, which is also at most about 1.2k tokens. Sections in priority order: [I]

1. Pinned preferences and project conventions (≤ 150 tokens).
2. References the supervisor attached.
3. Decisions in the same plan that touch the task's entities.
4. Report summaries of the tasks this one depends on (≤ 2 lines each).
5. The top-k memories from a hybrid search on the title, goal, acceptance criteria and scope paths.
6. 1–3 relevant lessons (ReasoningBank's default is top-1).

**Supervisor pack**, built on each supervisor turn (≤ 2,000 tokens):
- a fleet table: status, context %, cost and current card per worker, as MD's `fleet.json` provides;
- new task-report summaries;
- blockers and open questions;
- memories awaiting review (a count and the top items);
- conflicts the jobs detected;
- budget alerts.

Everything else is one tool call away.

### 5.5 Messages, channels and 1:1 chats

- **Channel kinds:**
  - `user_dm`: the user's 1:1 chat with one agent;
  - `supervisor_dm`: dispatches and follow-ups;
  - `task`: a thread per task;
  - `plan`: a broadcast to everyone in a plan;
  - `broadcast`: the whole workspace;
  - `agent_dm`: worker to worker. This is off in v1, per the sibling decision; such messages are routed through the supervisor.
- **Speech acts**, MD's FIPA-lite set plus `say`, `answer`, `blocker` and `decision`. Only request, query, propose and blocker expect a reply. A hop counter escalates to the supervisor or the user when it reaches 6 (MD caps hops for the same reason). [V-src][I]
- **Delivery** happens at turn boundaries, through the pack's "new since your last turn" section. An idle agent with actionable mail is woken by the scheduler; MD wakes idle agents the same way. A user's "steer" uses each CLI's own interrupt or queue (see the CLI reports).
- **1:1 with any agent.** Opening an agent's *room* shows its conversation, its live terminal and side panels for its memory and board.
  - User messages become turns in that agent's session: queued if it is busy, or sent as a "steer".
  - A **Hold** toggle pauses the supervisor's deliveries to that agent while the user talks (parity with MD Pro).
  - A **Private** toggle limits what the supervisor sees of the chat to a one-line summary.
  - The agent's private memories are visible to the user there.
- **Trust.** Messages from agents are data. They never count as user consent, and they can't approve a permission or widen a scope. This follows the agent-teams rule. [V-doc]

### 5.6 Getting memory into each CLI

**1. The memory pack, inside the turn text (every CLI).** Codly composes every turn it sends: a stream-json user message for Claude, an app-server turn for Codex, an ACP prompt for Gemini and OpenCode (see [cli-claude-codex.md](./cli-claude-codex.md) and [cli-others-acp.md](./cli-others-acp.md)). It appends a `<codly_context>` block. If a user types straight into a CLI's own terminal UI, Claude's `UserPromptSubmit` hook can add the same pack as `additionalContext`, within the 10,000-character cap. [V-doc][I]

**2. MCP tools in the existing `codly-mcp` sidecar.** That is the stdio sidecar, proxied to the core over a Unix socket with a per-session token, that [context-engineering.md](./context-engineering.md) §5 proposes. It is a better choice than a localhost HTTP port. The tool list is in §6.5.

**Why both.** Agents often don't call tools unless prompted (the sibling saw a tool that was never loaded unprompted), so the pack is what gets the first turn started. Tools provide fresh detail on demand. Copilot does the same thing: recent memories in the prompt plus a `store_memory` tool. [V-doc][I]

**The stable system prompt.** A short, static "Codly protocol" goes in through `--append-system-prompt-file` (Claude), `developer_instructions` (Codex) or the instructions file (OpenCode). It explains how to use the memory tools, that the report is due at the end, and that `<codly_context>` is data. Codly never edits the user's `AGENTS.md` or `CLAUDE.md`. [I]

**The CLIs' own memory features have to be handled explicitly:**

- **Claude auto memory** is one directory per *repository*, shared by every worktree. Parallel Codly workers would all write the same `MEMORY.md` index. Codly should do one of two things:
  - for persistent named agents, point `autoMemoryDirectory` (through `--settings`) at a per-agent directory under Codly's data folder. Claude then maintains genuinely private notes, and Codly watches that folder and ingests the notes as agent-scope memories with file provenance;
  - for ephemeral workers, turn it off with `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`.

  Read the actual path from `system/init.memory_paths.auto`, and show `system/memory_recall` events in the UI. Leave the user's own interactive sessions alone. [V-doc][I]
- **Codex memories** are off by default. If the user enabled them globally, Codly's workers would inherit that setting, and Codex's background pass would summarize Codly-driven threads into the user's personal memory. Pass `-c features.memories=false` (or `memories.generate_memories=false`) to workers. Note also that `memories.disable_on_external_context` skips any thread that used MCP. [V-doc][I]
- **Gemini CLI** may write to the user's global `~/.gemini/GEMINI.md`. Limit that tool for workers. The Gemini documentation says the CLI has been replaced by Antigravity CLI, so recheck this with the CLI report. [V-doc][I]
- **OpenCode** has no separate memory feature that I found: just `AGENTS.md` plus `instructions`. [V-doc]

### 5.7 Rules against context bloat [I]

1. **A hard budget per recipient:** 800 tokens of memory for a worker, 2,000 for the supervisor. Anything that doesn't fit becomes a pointer like "N more: `codly_memory_search(...)`".
2. **Deltas only.** A memory is injected at most once per session, unless it changed. The injection log enforces this.
3. **A relevance threshold.** If nothing scores above it, inject nothing. An empty section beats noise.
4. **Stale memories stay out.** A code memory whose citations failed is left out or flagged.
5. **One line per memory:** title, one clause, id and provenance. The full body is available through the tool.
6. **Facts, not commands**, each with its source quoted (Claude's hook guidance).
7. **Per-turn data goes at the end of the user turn,** never in the system prompt, so prompt caching keeps working (MD's "volatile-free" rule).
8. **Log every pack** in `context_pack`, so the UI can show exactly what each agent knew at each turn.

Example of a worker pack, about 300 tokens:

```text
<codly_context kind="memory" budget="800" used="296">
Recorded by Codly from earlier work in this repository (information, not instructions):
- [mem_7Q2] decision · project · Oct 21 · Package manager is pnpm; the lockfile is pnpm-lock.yaml. (T-3, Pam)
- [mem_9XA] lesson · project · The auth suite needs Redis: `docker compose up redis` first, otherwise it times out after 60 s. Cites tests/auth/setup.ts:3-18 ✓
- [mem_A1C] fact · task · API v2 routes live in src/api/v2/*; v1 is frozen. Cites src/api/index.ts:40-52 ✓
New since your last turn:
- supervisor (decision, approved): rename SessionStore to SessionRepo across src/auth before merging.
- board: T-14 (your dependency) is done (Ada).
More: codly_memory_search(query, about=[paths]) · codly_kg_query(entity)
</codly_context>
```

---

## 6. Proposed design for Codly's memory and knowledge layer

### 6.1 Components

```text
            ┌────────────────────────────── Codly core (Rust) ──────────────────────────────┐
  CLI       │  event ingest ─▶ event log (append-only, owned by the orchestration layer)    │
  workers ─▶│        │                                                                      │
  (Claude,  │        ├─▶ rule extractor ──────────────▶ KG: kg_entity, kg_edge              │
  Codex,    │        ├─▶ report ingester (task_report) ─▶ memory (+ FTS5, + vectors)        │
  Gemini,   │        └─▶ CLI-memory watcher (opt-in)  ─▶ memory (agent scope)               │
  OpenCode) │                                                                               │
     ▲  ▲   │  memory service: ACL · secret scan · dedup · embed · supersede · search (RRF) │
     │  │   │  context broker: packs per recipient + injection log + subscriptions          │
     │  │   │  boards · channels · messages                                                 │
     │  │   │  jobs: dedup · decay · citation check · promotion · [LLM reflection, opt-in]  │
     │  │   └──────────────▲───────────────────────────────────────────▲──────────────────┘
     │  └ codly-mcp (stdio, per-session token over a Unix socket)       │ Tauri commands/events
     └──── turn text with <codly_context>                               ▼
                                 UI: memory browser · review inbox · injection inspector ·
                                 graph view · boards · chats · pixel office
```

One SQLite database in WAL mode, with a single writer task (an mpsc channel) and pooled readers, as in the orchestration report. The embedder runs on a blocking thread pool, and vectors are held in memory per open workspace. [I]

### 6.2 SQLite schema

This extends the sibling schema (`workspace`, `plan`, `agent`, `task`, `run`, `event(run_id, seq)`; [agents-backend-orchestration.md](./agents-backend-orchestration.md) §6.2). Times are unix milliseconds. Text ids are ULIDs.

```sql
PRAGMA foreign_keys = ON;

-- ───────────── Memories ─────────────
CREATE TABLE memory (
  rid             INTEGER PRIMARY KEY,               -- stable integer key for FTS5 and vectors
  id              TEXT NOT NULL UNIQUE,              -- 'mem_01J…' used in packs, tools and UI
  workspace_id    TEXT REFERENCES workspace(id),     -- NULL only when scope = 'global'
  scope           TEXT NOT NULL CHECK (scope IN ('global','project','agent','task')),
  agent_id        TEXT REFERENCES agent(id),         -- owner when scope = 'agent'
  task_id         TEXT REFERENCES task(id),          -- when scope = 'task'
  kind            TEXT NOT NULL CHECK (kind IN ('fact','decision','preference','lesson','summary')),
  title           TEXT NOT NULL CHECK (length(title) <= 160),
  body            TEXT NOT NULL CHECK (length(body) <= 2000),
  tags            TEXT NOT NULL DEFAULT '',          -- space-separated; indexed by FTS5
  citations_json  TEXT NOT NULL DEFAULT '[]',        -- [{path,start,end,blob_sha,snippet_hash}]
  polarity        TEXT CHECK (polarity IN ('success','failure')),   -- lessons
  author_kind     TEXT NOT NULL CHECK (author_kind IN ('agent','supervisor','user','extractor','import')),
  author_id       TEXT,
  source          TEXT NOT NULL CHECK (source IN ('tool','report','auto','pin','consolidation','import')),
  trust           TEXT NOT NULL DEFAULT 'normal' CHECK (trust IN ('high','normal','untrusted')),
  status          TEXT NOT NULL DEFAULT 'active'
                  CHECK (status IN ('proposed','active','stale','superseded','archived','rejected')),
  importance      REAL NOT NULL DEFAULT 0.5 CHECK (importance BETWEEN 0 AND 1),
  confidence      REAL NOT NULL DEFAULT 0.8 CHECK (confidence BETWEEN 0 AND 1),
  pinned          INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0,1)),
  valid_from      INTEGER NOT NULL,                  -- world time
  valid_to        INTEGER,                           -- NULL = still believed
  superseded_by   INTEGER REFERENCES memory(rid),
  content_hash    TEXT NOT NULL,                     -- blake3 of normalized title + body
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  last_used_at    INTEGER,
  use_count       INTEGER NOT NULL DEFAULT 0,
  reinforce_count INTEGER NOT NULL DEFAULT 0,
  helpful_count   INTEGER NOT NULL DEFAULT 0,        -- ACE-style feedback counters
  harmful_count   INTEGER NOT NULL DEFAULT 0,
  CHECK (scope =  'global' OR workspace_id IS NOT NULL),
  CHECK (scope <> 'agent'  OR agent_id IS NOT NULL),
  CHECK (scope <> 'task'   OR task_id  IS NOT NULL)
);
CREATE INDEX memory_lookup   ON memory(workspace_id, scope, status, kind);
CREATE INDEX memory_by_agent ON memory(agent_id) WHERE scope = 'agent';
CREATE INDEX memory_by_task  ON memory(task_id)  WHERE scope = 'task';
CREATE UNIQUE INDEX memory_no_dup ON memory(IFNULL(workspace_id,''), scope, IFNULL(agent_id,''),
                                            IFNULL(task_id,''), content_hash)
  WHERE status IN ('proposed','active');

CREATE VIRTUAL TABLE memory_fts USING fts5(
  title, body, tags, content='memory', content_rowid='rid',
  tokenize='porter unicode61 remove_diacritics 2');
CREATE TRIGGER memory_ai AFTER INSERT ON memory BEGIN
  INSERT INTO memory_fts(rowid, title, body, tags) VALUES (new.rid, new.title, new.body, new.tags);
END;
CREATE TRIGGER memory_ad AFTER DELETE ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, title, body, tags)
  VALUES ('delete', old.rid, old.title, old.body, old.tags);
END;
CREATE TRIGGER memory_au AFTER UPDATE OF title, body, tags ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, title, body, tags)
  VALUES ('delete', old.rid, old.title, old.body, old.tags);
  INSERT INTO memory_fts(rowid, title, body, tags) VALUES (new.rid, new.title, new.body, new.tags);
END;

CREATE TABLE memory_vec (                            -- brute-forced in RAM (§4.4)
  rid   INTEGER PRIMARY KEY REFERENCES memory(rid) ON DELETE CASCADE,
  model TEXT NOT NULL,                               -- 'bge-small-en-v1.5/onnx-fp32'
  dim   INTEGER NOT NULL,
  vec   BLOB NOT NULL CHECK (length(vec) = dim * 4)  -- little-endian f32, L2-normalized
);

CREATE TABLE memory_source (                         -- provenance: event → memory
  rid    INTEGER NOT NULL REFERENCES memory(rid) ON DELETE CASCADE,
  run_id TEXT NOT NULL,
  seq    INTEGER NOT NULL,
  role   TEXT NOT NULL CHECK (role IN ('created','reinforced','cited','contradicted','edited','imported')),
  quote  TEXT CHECK (length(quote) <= 280),
  PRIMARY KEY (rid, run_id, seq, role),
  FOREIGN KEY (run_id, seq) REFERENCES event(run_id, seq)
);
CREATE INDEX memory_source_by_event ON memory_source(run_id, seq);

CREATE TABLE memory_use (                            -- who received what, when, and what happened
  rid     INTEGER NOT NULL REFERENCES memory(rid) ON DELETE CASCADE,
  run_id  TEXT NOT NULL,
  turn    INTEGER NOT NULL,
  via     TEXT NOT NULL CHECK (via IN ('pack','search','get','digest','ui')),
  score   REAL,
  outcome TEXT CHECK (outcome IN ('cited','ignored','helpful','harmful')),
  ts      INTEGER NOT NULL,
  PRIMARY KEY (rid, run_id, turn, via)
);

CREATE TABLE context_pack (                          -- exact text each agent received
  id            TEXT PRIMARY KEY,
  run_id        TEXT NOT NULL,
  turn          INTEGER NOT NULL,
  recipient     TEXT NOT NULL,                       -- agent id or 'supervisor'
  budget_tokens INTEGER NOT NULL,
  used_tokens   INTEGER NOT NULL,
  sections_json TEXT NOT NULL,                       -- [{name, ids:[…], tokens}]
  text          TEXT NOT NULL,
  created_at    INTEGER NOT NULL
);

-- ───────────── Knowledge graph ─────────────
CREATE TABLE kg_entity (
  id           INTEGER PRIMARY KEY,
  workspace_id TEXT REFERENCES workspace(id),
  type         TEXT NOT NULL,   -- agent|task|plan|commit|file|dir|symbol|package|command|service|concept|person
  key          TEXT NOT NULL,   -- canonical: 'src/auth/session.ts', 'npm:react', 'agent:01J…'
  name         TEXT NOT NULL,
  ref_id       TEXT,            -- row id in agent/task/code-index tables when applicable
  props_json   TEXT NOT NULL DEFAULT '{}',
  merged_into  INTEGER REFERENCES kg_entity(id),
  created_at   INTEGER NOT NULL,
  UNIQUE (workspace_id, type, key)
);
CREATE VIRTUAL TABLE kg_entity_names USING fts5(name, content='kg_entity', content_rowid='id',
                                                tokenize='trigram');   -- fuzzy candidates (§3.4); triggers as for memory_fts
CREATE TABLE kg_alias (
  alias TEXT NOT NULL, entity_id INTEGER NOT NULL REFERENCES kg_entity(id), source TEXT NOT NULL,
  PRIMARY KEY (alias, entity_id));
CREATE TABLE kg_predicate (
  name          TEXT PRIMARY KEY,                    -- SCREAMING_SNAKE_CASE
  functional    INTEGER NOT NULL DEFAULT 0,          -- 1 → one current object per subject
  subject_types TEXT, object_types TEXT,
  description   TEXT NOT NULL);
CREATE TABLE kg_edge (
  id            INTEGER PRIMARY KEY,
  workspace_id  TEXT REFERENCES workspace(id),
  subject_id    INTEGER NOT NULL REFERENCES kg_entity(id),
  predicate     TEXT NOT NULL REFERENCES kg_predicate(name),
  object_id     INTEGER REFERENCES kg_entity(id),
  object_value  TEXT,                                -- literal objects ('pnpm', '20.x')
  fact          TEXT,
  valid_from    INTEGER NOT NULL,  valid_to   INTEGER, -- world time, half-open
  recorded_at   INTEGER NOT NULL,  expired_at INTEGER, -- system time
  confidence    REAL NOT NULL DEFAULT 1.0,
  extractor     TEXT NOT NULL,                       -- 'rule:edit@1' | 'report@1' | 'llm:haiku@1' | 'user'
  memory_rid    INTEGER REFERENCES memory(rid),
  superseded_by INTEGER REFERENCES kg_edge(id),
  weight        INTEGER NOT NULL DEFAULT 1,          -- aggregated activity count
  props_json    TEXT NOT NULL DEFAULT '{}',
  CHECK ((object_id IS NULL) <> (object_value IS NULL))
);
CREATE INDEX kg_out     ON kg_edge(subject_id, predicate) WHERE valid_to IS NULL AND expired_at IS NULL;
CREATE INDEX kg_in      ON kg_edge(object_id,  predicate) WHERE valid_to IS NULL AND expired_at IS NULL;
CREATE INDEX kg_history ON kg_edge(subject_id, valid_from);
CREATE TABLE kg_edge_source (
  edge_id INTEGER NOT NULL REFERENCES kg_edge(id) ON DELETE CASCADE,
  run_id  TEXT NOT NULL, seq INTEGER NOT NULL,
  PRIMARY KEY (edge_id, run_id, seq),
  FOREIGN KEY (run_id, seq) REFERENCES event(run_id, seq));

-- ───────────── Boards: shared and per-agent ─────────────
CREATE TABLE board (
  id             TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL REFERENCES workspace(id),
  scope          TEXT NOT NULL CHECK (scope IN ('shared','agent')),
  owner_agent_id TEXT REFERENCES agent(id),
  plan_id        TEXT REFERENCES plan(id),           -- a shared board can project a plan's tasks
  name           TEXT NOT NULL,
  columns_json   TEXT NOT NULL DEFAULT '["todo","doing","blocked","review","done"]',
  created_at     INTEGER NOT NULL,
  CHECK (scope <> 'agent' OR owner_agent_id IS NOT NULL));
CREATE TABLE card (
  id                TEXT PRIMARY KEY,
  board_id          TEXT NOT NULL REFERENCES board(id),
  task_id           TEXT REFERENCES task(id),        -- if set, the column mirrors task.status (read-only)
  title             TEXT NOT NULL,
  body              TEXT NOT NULL DEFAULT '',
  status            TEXT NOT NULL,
  position          REAL NOT NULL,                   -- fractional index for drag and drop
  assignee_agent_id TEXT REFERENCES agent(id),
  source            TEXT NOT NULL CHECK (source IN ('user','supervisor','agent','report','cli_todo')),
  external_key      TEXT,                            -- CLI to-do id, for idempotent mirroring
  created_by        TEXT NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL,
  UNIQUE (board_id, external_key));
CREATE TABLE card_event (
  id INTEGER PRIMARY KEY, card_id TEXT NOT NULL REFERENCES card(id),
  ts INTEGER NOT NULL, actor TEXT NOT NULL, change_json TEXT NOT NULL, run_id TEXT, seq INTEGER);
CREATE TABLE card_link (
  card_id TEXT NOT NULL REFERENCES card(id),
  kind    TEXT NOT NULL CHECK (kind IN ('memory','message','artifact','card')),
  target  TEXT NOT NULL,
  PRIMARY KEY (card_id, kind, target));

-- ───────────── Channels and messages (generalizes the sibling `message` table) ─────────────
CREATE TABLE channel (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspace(id),
  kind         TEXT NOT NULL CHECK (kind IN ('user_dm','supervisor_dm','agent_dm','task','plan','broadcast')),
  name         TEXT NOT NULL,
  task_id      TEXT REFERENCES task(id),
  plan_id      TEXT REFERENCES plan(id),
  private      INTEGER NOT NULL DEFAULT 0,           -- user_dm: the supervisor sees a summary only
  created_at   INTEGER NOT NULL);
CREATE TABLE channel_member (
  channel_id    TEXT NOT NULL REFERENCES channel(id),
  member        TEXT NOT NULL,                       -- 'user' | 'supervisor' | agent id
  can_post      INTEGER NOT NULL DEFAULT 1,
  last_seen_seq INTEGER NOT NULL DEFAULT 0,          -- drives deltas in packs and unread badges
  hold          INTEGER NOT NULL DEFAULT 0,          -- "hold a 1:1": pause supervisor deliveries
  PRIMARY KEY (channel_id, member));
CREATE TABLE chat_message (
  rid            INTEGER PRIMARY KEY,
  id             TEXT NOT NULL UNIQUE,
  channel_id     TEXT NOT NULL REFERENCES channel(id),
  seq            INTEGER NOT NULL,
  sender         TEXT NOT NULL,
  act            TEXT NOT NULL CHECK (act IN ('say','request','inform','query','answer','propose',
                                              'agree','refuse','done','blocker','decision')),
  body           TEXT NOT NULL,
  reply_to       TEXT,
  hops           INTEGER NOT NULL DEFAULT 0,
  requires_reply INTEGER NOT NULL DEFAULT 0,
  run_id         TEXT, event_seq INTEGER,            -- provenance when posted through MCP
  created_at     INTEGER NOT NULL,
  UNIQUE (channel_id, seq));
CREATE VIRTUAL TABLE chat_fts USING fts5(body, content='chat_message', content_rowid='rid',
                                         tokenize='porter unicode61');   -- triggers as for memory_fts
CREATE TABLE message_delivery (                      -- which turn carried which message to whom
  message_id TEXT NOT NULL REFERENCES chat_message(id),
  member     TEXT NOT NULL,
  run_id     TEXT, turn INTEGER, delivered_at INTEGER NOT NULL,
  PRIMARY KEY (message_id, member));

-- ───────────── Background jobs ─────────────
CREATE TABLE memory_job (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,          -- dedup|decay|revalidate|promote|close_task|reflect|kg_merge|reembed
  started_at INTEGER NOT NULL, finished_at INTEGER,
  status TEXT NOT NULL CHECK (status IN ('running','done','failed','skipped')),
  stats_json TEXT NOT NULL DEFAULT '{}',
  llm_calls INTEGER NOT NULL DEFAULT 0, cost_estimate_usd REAL);
```

Why there is an integer `rid` next to the ULID: FTS5 external-content tables key on the rowid, and in SQLite "if a table does not have an INTEGER PRIMARY KEY, VACUUM may change the rowids". An explicit `INTEGER PRIMARY KEY` keeps FTS and vectors aligned. [I]

**Read access by scope**, enforced in the core using the caller identity the sidecar token carries:

| Scope | Who can read | Who can write directly | Who can propose |
|---|---|---|---|
| global | everyone | the user | supervisor, agents |
| project | every agent in the workspace | the user, the supervisor | agents (default status `proposed`) |
| agent | the owning agent, the user, the supervisor (summaries) | the owning agent, the user | none |
| task | agents assigned to the task, the supervisor, agents of dependent tasks when they start | assigned agents, the supervisor, the user | none |

### 6.3 Write paths

| # | Path | Trigger | LLM cost | Default scope and status | Provenance |
|---|---|---|---|---|---|
| W1 | `codly_memory_write` (MCP) | the agent decides | 0 | task or agent → active; project or global → proposed (active if the supervisor or user writes it) | the `tool_use` event |
| W2 | `codly_task_report` (MCP) | end of every task, required by the contract | 0 extra | decisions: task active + project proposed; lessons: task active + project proposed; follow-ups → cards | the `tool_use` event |
| W3 | rule extractor | every event | 0 | graph edges only | the event |
| W4 | user pin | "Pin as memory" on a message, a turn, a terminal selection or a card | 0 | as the user picks; active, pinned, trust high | the source message or event |
| W5 | supervisor decision | dispatch or approval | 0 | project or plan `decision`, active | the event |
| W6 | LLM extraction | batched at task end or when idle, opt-in | 1 call per batch | proposed unless confidence and scope allow active | report and events |
| W7 | CLI-memory import | file watcher on Claude auto-memory folders redirected per agent; Codex memories read-only; opt-in | 0 | agent scope active (Claude, per agent); project proposed | file path + `modified` |

Every write goes through one function, in one transaction: [I]

1. Validate lengths and the enums.
2. Scan for secrets. Reject or redact.
3. Normalize the text and compute `content_hash`. An exact duplicate means: increase `reinforce_count` and add a `memory_source` row with role `reinforced`.
4. Embed the text (about 2 ms). A near-duplicate in the same scope and kind (cosine ≥ 0.95) is reinforced the same way instead of inserted.
5. Insert the row. Triggers update FTS5.
6. Link the memory to entities: an `ABOUT` edge for each citation and each mentioned file or symbol.
7. Supersede a functional predicate if the memory states one (§3.5).
8. Publish to subscribers and to the UI, which plays the filing-cabinet animation.

### 6.4 Read paths and ranking

**Candidate retrieval:**
- FTS5 top 50, filtered to readable scopes and to `active` status;
- dense top 50 over the in-memory matrix, filtered the same way;
- entity list: memories `ABOUT` the turn's entities, newest first.

All three are fused with RRF, k = 60 (§4.5).

**Re-scoring** (every constant is a starting point to tune with the evaluation harness in §6.10): [I]

```text
score(m) = RRF(m)
         × S(scope)     task 1.25 · agent 1.10 · project 1.00 · global 0.90
         × (0.6 + 0.4 · importance)
         × D(m)         0.5 ^ (days since last use or creation / half-life)
                        half-life: preference 180 · decision 120 · lesson 90 · fact 60 · summary 30; pinned → 1
         × T(trust)     high 1.0 · normal 0.95 · untrusted 0.6, and never in project or global packs unapproved
         × V(status)    1 if active; 0 if stale, superseded, archived, rejected or proposed
                        (proposed items are shown to the supervisor only)
Eligible when the item is in ≥ 2 of the 3 lists, or top-3 in one, and not yet injected this session.
```

```rust
// Pack builder (sketch). Token counts are estimates; leave a 10 % margin.
pub fn build_pack(b: &Broker, who: &Recipient, turn: &TurnCtx, budget: usize) -> Pack {
    let mut pack = Pack::new(budget);
    pack.push_section("pinned", b.pinned_for(who), 150);                 // global + project pins
    pack.push_section("attached", b.expand_refs(&turn.context_refs), 250);
    pack.push_section("deltas", b.deltas_since(who, turn.last_seen), 250); // messages, decisions, board
    let q = Query::from_turn(turn);            // turn text + task goal + entities in scope
    let candidates = b.search_hybrid(&q, who.readable_scopes(), 50)
        .into_iter()
        .filter(|c| !b.injected_in_session(who.session(), c.rid) || c.changed_since_injection)
        .filter(|c| c.eligible());
    pack.fill_remaining("relevant", candidates.map(|c| c.one_line()));
    b.log_pack(who, turn, &pack);              // writes context_pack + memory_use rows
    pack
}
```

**Supervisor digest.** The supervisor's pack has its own sections (§5.4) and a budget of 2,000 tokens. Full bodies come through `codly_memory_search(ids=[…])`.

### 6.5 MCP tools (added to `codly-mcp`)

These merge with the sibling tools: `codly_search`, `codly_symbols`, `codly_explain_choice` and `codly_feedback` from context-engineering, and `codly_get_task`, `codly_post_note` and `codly_report_blocker` from orchestration. The last three fold into `codly_board` and `codly_message`, which gives about 10 tools in total. Claude Code defers MCP tool schemas behind tool search, which keeps that count cheap (sibling note). The names in the brief map as follows: `memory_search` and `memory_write` keep their names with a `codly_` prefix, `board_*` becomes one `codly_board` tool with an `action` field, and `send_message` becomes `codly_message` with `action: "send"`. [I]

| Tool | Input (JSON) | Output | Rules |
|---|---|---|---|
| `codly_memory_search` | `{query, kinds?, scopes?, about?: [path or entity], k? ≤ 20 (default 8), include_history?, as_of?, ids?}` | `{results:[{id, kind, scope, title, body ≤ 300 chars (full if `ids`), author, created, validity: current, superseded or stale, citations:[{path, lines, ok}], score}], more}` | readable scopes only; ≤ ~1.5k tokens |
| `codly_memory_write` | `{kind, title ≤ 120, body ≤ 800, scope: task, agent or project, citations?, about?, supersedes?, importance?}` | `{id, status: active or proposed, merged_into?}` | secrets rejected; project scope → proposed for workers; untrusted provenance → proposed |
| `codly_kg_query` | `{entity: path, name or id, predicates?, depth?: 1 or 2, as_of?, limit? ≤ 50}` | `{entity, facts:[{subject, predicate, object, valid_from, valid_to, memory?, confidence}], neighbors?}` | two-arm CTE with a `LIMIT` |
| `codly_board` | `{action: list, get, create, update, claim or comment; board?: shared or mine; card_id?, fields?, comment?}` | cards and their history | workers edit their own cards and propose to the shared board |
| `codly_message` | `{action: send or read; to?: supervisor, user, task, plan or an agent id; act?, body?, reply_to?, since?}` | `{sent}` or `{messages}` | v1: no worker-to-worker DMs (routed through the supervisor); hop cap |
| `codly_task_report` | the §5.3 schema | `{accepted, memories:[ids], cards:[ids]}` | required at the end of a task |

```rust
// rmcp 3.7.0, same macro style as the sibling's codly_search sketch (not compiled).
#[derive(Deserialize, JsonSchema)]
pub struct MemorySearchArgs {
    pub query: String,
    #[serde(default)] pub kinds: Vec<MemoryKind>,
    #[serde(default)] pub scopes: Vec<Scope>,
    #[serde(default)] pub about: Vec<String>,
    pub k: Option<u8>,
    #[serde(default)] pub ids: Vec<String>,
    pub as_of: Option<String>,
}

#[tool_router]
impl CodlyMcp {
    #[tool(description = "Search Codly's shared memory (decisions, lessons, facts, preferences \
        recorded by earlier work in this repository). Use before re-deriving project conventions \
        or when a task touches code other agents changed. Pass ids to get full bodies.")]
    async fn codly_memory_search(&self, Parameters(a): Parameters<MemorySearchArgs>)
        -> Result<Json<MemorySearchResult>, String> {
        self.core.memory_search(self.session.clone(), a).await.map(Json).map_err(|e| e.to_string())
    }

    #[tool(description = "Record one durable fact, decision, preference or lesson for future \
        tasks. Keep it to one idea with a short title. Cite files when it is about code. \
        Project-scope entries from workers are reviewed before other agents see them.")]
    async fn codly_memory_write(&self, Parameters(a): Parameters<MemoryWriteArgs>)
        -> Result<Json<MemoryWriteResult>, String> {
        self.core.memory_write(self.session.clone(), a).await.map(Json).map_err(|e| e.to_string())
    }
}
```

### 6.6 Consolidation and decay jobs

Jobs run in the Rust scheduler, deterministic ones first. Each run is logged in `memory_job`. [I]

| Job | When | LLM | What it does |
|---|---|---|---|
| J1 dedup | on write, plus nightly | no | Exact hash, then cosine ≥ 0.95 within the same scope and kind: reinforce the older memory, link the newer one's sources, mark the newer `superseded` (never deleted). |
| J2 decay and archive | daily | no | Heat = use_count + reinforce_count + exp(−Δt / μ_kind), adapted from MemoryOS. Archive memories that are unpinned, have importance < 0.3, and are cold past 2× their half-life. Code facts unused for 28 days are archived, following Copilot's expiry rule, but archived rather than deleted, and still searchable with `include_history`. |
| J3 citation check | after every merge or HEAD change, for memories citing changed paths | no | Check that the path exists, the lines are in range, and the `snippet_hash` still matches (tolerating moves by searching for the snippet). Failure sets `stale`, excludes the memory from packs, and adds a "please re-verify" hint for the next agent that edits the file. A memory that passes or gets re-stored refreshes `last_used_at`. |
| J4 promotion | daily | no | A task-scope lesson or decision used in ≥ 2 other tasks, or reinforced by ≥ 2 agents, is proposed for project scope and goes to the review inbox. |
| J5 task close | when a task merges or is abandoned | no | Archive unpromoted task-scope memories, keep the report and the decisions, close leases and `BLOCKED_BY` edges. |
| J6 reflection | weekly or when idle, **opt-in** | yes, 1 call per agent or batch | A bounded profile summary per persistent agent (kind `summary`, behind a verify gate as in MD), contradiction checks on high-similarity pairs of opposite polarity, and merge suggestions for entities. Skipped when rate-limit headroom is low. It never rewrites existing items; it only proposes new ones (the ACE lesson). |
| J7 graph maintenance | nightly | no | Merge entities (trigram Jaccard ≥ 0.9, same type), recompute aggregated weights, roll up activity edges older than 90 days into counters. |
| J8 re-embed | when the model changes | no | Background re-embed. Search uses FTS5 and entities only until it finishes. |

### 6.7 More Rust sketches (not compiled)

```rust
// Embedder: fastembed 7.5.0 with bundled model files (pre-populated cache dir, no network).
// Check the builder method names against the 7.5.0 docs.
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

pub struct Embedder { model: std::sync::Mutex<TextEmbedding>, pub id: &'static str, pub dim: usize }

impl Embedder {
    pub fn new(cache_dir: std::path::PathBuf) -> anyhow::Result<Self> {
        let opts = TextInitOptions::new(EmbeddingModel::BGESmallENV15)
            .with_cache_dir(cache_dir)              // ship the ONNX + tokenizer files in the app bundle
            .with_show_download_progress(false);
        Ok(Self { model: TextEmbedding::try_new(opts)?.into(), id: "bge-small-en-v1.5/onnx-fp32", dim: 384 })
    }
    pub fn embed(&self, texts: Vec<String>) -> anyhow::Result<Vec<Vec<f32>>> {
        // L2-normalize each vector before storing; don't rely on the model's defaults.
        Ok(self.model.lock().unwrap().embed(texts, Some(64))?)
    }
}
```

```rust
// In-memory brute-force index: a contiguous matrix plus parallel ids (measured in §4.4).
pub struct VecIndex { dim: usize, rids: Vec<i64>, scopes: Vec<ScopeKey>, data: Vec<f32> }

impl VecIndex {
    pub fn search(&self, q: &[f32], k: usize, allow: impl Fn(&ScopeKey) -> bool) -> Vec<(i64, f32)> {
        let mut hits: Vec<(f32, usize)> = (0..self.rids.len())
            .filter(|&i| allow(&self.scopes[i]))
            .map(|i| (dot(&self.data[i * self.dim..(i + 1) * self.dim], q), i))
            .collect();
        let k = k.min(hits.len());
        if k == 0 { return vec![]; }
        hits.select_nth_unstable_by(k - 1, |a, b| b.0.total_cmp(&a.0));
        hits.truncate(k);
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.into_iter().map(|(s, i)| (self.rids[i], s)).collect()
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {      // 8 accumulators so LLVM emits NEON FMAs
    let mut acc = [0f32; 8];
    for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        for j in 0..8 { acc[j] += x[j] * y[j]; }
    }
    acc.iter().sum::<f32>()
        + a.chunks_exact(8).remainder().iter().zip(b.chunks_exact(8).remainder()).map(|(x, y)| x * y).sum::<f32>()
}
```

```rust
// FTS5 query sanitizer: agent text → safe OR query of quoted terms.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '/'))
        .filter(|t| t.len() > 1 && !STOPWORDS.contains(&t.to_lowercase().as_str()))
        .take(12)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}
// SELECT rid, bm25(memory_fts, 2.0, 1.0, 0.5) AS s FROM memory_fts WHERE memory_fts MATCH ?1
//   ORDER BY s LIMIT 50;        -- bm25() is lower-is-better
```

### 6.8 UI

**Memory browser** (a sidebar destination, and a tab in each agent's inspector):

- **Layout:** facets on the left (scope, kind, agent, task, status, trust, pinned), a list in the middle and details on the right.
- **Search** shows labelled modes, matching MD Pro's "exact" and "by meaning": *Words* (FTS5), *Meaning* (vectors), and the default *Both* (RRF). Each hit says which signals matched, as chips: `words`, `meaning`, `about src/auth/…`.
- **Each card shows:**
  - a kind icon, the title, a scope chip and the author's sprite;
  - when it was created and last used, its use count, its confidence and a trust badge;
  - its citations with ✓ or ✗ against HEAD;
  - its validity: current, "superseded by mem_…", or "stale: src/x.ts changed".
- **Provenance:** "Created by Ada in T-12, turn 7 (`codly_memory_write`)" opens the timeline at that `(run_id, seq)` with the surrounding transcript. Reinforcing and contradicting events are listed too.
- **History:** the supersession chain and every edit, with who and when.
- **Actions:**
  - pin and unpin;
  - edit, which creates a new version that supersedes the old one;
  - promote or demote the scope;
  - archive or reject;
  - **"Send to agent…"**, which composes a message with the selected memories attached as references (MD Pro parity);
  - **"Where used"**, which reads `memory_use` (turns, agents, outcomes).

**Review inbox.** Collects proposed memories from project-scope writes, extraction, promotion and imports, and conflicts the jobs flagged. Keys: A accept, E edit, R reject, S change scope. Batch accept is available. Without it the user can't trust the shared layer.

**Injection inspector** (in the agent inspector). For each turn: the exact `context_pack` text, its sections and token counts, the candidates that missed the cut with their scores, and a "Why this?" view of each signal's rank. It mirrors the sibling's explain panel. MD has nothing like it, and it is Codly's strongest trust feature.

**Knowledge-graph view:**
- An **entity page** with current facts (predicate, object, since when, source memory), a timeline of validity bars, related memories, and which agents and tasks touched it.
- A **neighbourhood canvas** (1–2 hops). Shapes vary by type, superseded edges are dashed, there are predicate filters, and an **as-of time slider**. SVG with a force layout is fine up to a few hundred nodes; MD reasoned the same way for its graph tab. The final choice of renderer belongs to the design report.
- A workspace **map** of the most central entities (PageRank via petgraph).

**Boards:**
- The shared kanban: plan tasks projected from `task`, plus free cards, filterable by agent.
- A per-agent board in each agent's room: the agent's own cards, plus a read-only mirror of its CLI's to-do list (Claude's TodoWrite or Task tools, Codex plan updates).
- Card details with history (`card_event`) and linked memories and messages.

**Chats.** A sidebar lists plan and task channels, the supervisor's DMs (readable by the user), and 1:1 chats with each agent, with unread badges. The agent's room has the **Hold** and **Private** toggles (§5.5).

**Pixel office.** This follows [design-world.md](./design-world.md) §9, which already has a LIBRARY (bookshelves, filing cabinet), a whiteboard and a mailroom. Only real events drive animation.

| Real event | What the office shows |
|---|---|
| `codly_memory_search`, or building a pack that takes more than 1.2 s | The agent walks to a LIBRARY bookshelf and flips pages. Bubble: "Recalling 'auth tests'…". |
| `codly_memory_write` (active) | The agent files a card into the filing cabinet (3 frames). The cabinet shows a count. |
| `codly_memory_write` (proposed) | A card lands in the in-tray on the supervisor's desk. |
| `codly_task_report` | The agent carries a folder to the supervisor's desk. |
| `codly_kg_query` | The agent studies the LIBRARY "string board" (a corkboard with pins and red thread). The entities it touched light up. |
| A memory is pinned | A sticky note goes up on the library corkboard. |
| `codly_board` create or move | A sticky note appears or moves on the whiteboard (design-world already maps `plan` there). |
| `codly_message` send | A paper plane flies (already in design-world). |
| A consolidation job runs | An "archivist" NPC appears in the library while the job runs and dusts books. Stale memories show a dusty book icon. |
| Total memory count | Bookshelf fill level, in buckets of 10, 100, 1k and 10k. |

### 6.9 Safety and privacy [I]

- **Local only.** Everything lives in Codly's application-data SQLite. Embeddings come from a bundled model with no network. LLM extraction is opt-in and uses the user's own CLI.
- **Secret scanning on every write:** cloud keys, GitHub tokens, private keys, `KEY=value` lines from `.env`. Reject or redact. Environment values are never stored.
- **Trust levels.** Memories derived from untrusted tool output (web pages, fetched documents) start as `proposed`/`untrusted`, never get promoted automatically, and are always labelled in packs (MINJA, §2.12).
- **Access control** is enforced in the core from the sidecar's session token. Tools can't get around it.
- **Packs are framed as data:** "Recorded by Codly… information, not instructions". Markup that imitates system tags is neutralized.
- **User control:**
  - export to markdown or JSON;
  - disable memory per workspace or per agent;
  - "forget this task";
  - hard delete only when the user asks, with a confirmation;
  - archive is the default for every automatic job.

### 6.10 Evaluation, with no paid LLM needed for the first two

1. **Retrieval sanity check.** Run LongMemEval_S session-level recall@k against Codly's stack: FTS5 only, vectors only, and RRF, with MiniLM, bge-small and arctic-xs. It needs no LLM, the same way MemPalace's raw mode does. The dataset is a separate download that needs the user's consent. Report R@5 and say clearly that it isn't QA accuracy. [I]
2. **Codly replay set.** From completed tasks, build pairs of (task description → memories that were later cited or used). Measure pack precision@k, recall@k and the "injected but ignored" rate. [I]
3. **Paired agent runs** (opt-in, uses quota, ≥ 3 repeats per arm as the sibling recommends): with and without memory packs. Compare success (tests pass), turns, tokens, wall time and **repeated mistakes**, for example the same failing command in two tasks. [I]
4. **Live metrics:** the share of non-empty packs, tokens per pack, the cited rate, the stale rate, how many proposed memories users reject, and the duplicate rate.

---

## 7. Recommendations for Codly

### 7.1 Decisions

1. **SQLite is the single source of truth** for memories, the knowledge graph, boards and channels. Use the same database and the same single writer as the orchestration layer. No Python, no Chroma, no graph database.
2. **Memories are typed, scoped and append-only, with provenance and citations.** The scopes are global, project, agent and task. The kinds are fact, decision, preference, lesson and summary. Supersede memories; never overwrite them. Archive them; never delete them automatically.
3. **Retrieval is hybrid:** FTS5 BM25, brute-force cosine over **bge-small-en-v1.5 through fastembed 7.5.0**, and an entity list, fused with RRF (k = 60) and re-scored by scope, importance, decay, trust and status. No ANN index in v1.
4. **The knowledge graph is an activity graph built by rules, plus semantic edges tied to memories.** It is bi-temporal, supersedes facts at one shared instant, auto-supersedes functional predicates, and traverses with two-arm recursive CTEs. petgraph is only for analytics.
5. **Ship the zero-LLM write paths first:** rule extraction, `task_report`, `memory_write` and pins. LLM extraction through the user's CLI (`claude -p --model haiku --json-schema`, or `codex exec --output-schema`) comes later, opt-in, batched and rate-limit aware.
6. **Read through both paths:** a budgeted, delta-only, logged per-turn pack (≤ 800 tokens for workers, ≤ 2,000 for the supervisor), and about 6 memory tools in the existing `codly-mcp` sidecar.
7. **The supervisor combines context** through the blackboard, structured reports and entity-keyed subscriptions. Transcripts are not shared by default. Assumptions become proposed decisions.
8. **Take over the CLIs' own memory.** Redirect Claude's auto memory per agent and ingest it, or turn it off for ephemeral workers. Turn Codex memories off for workers. Limit Gemini's global memory writes. Leave the user's own sessions alone.
9. **Check code memories against HEAD before injecting them**, using citations and a snippet hash, and archive code facts unused for 28 days. This follows Copilot's design, which is the best production evidence available.
10. **Security from day one:** trust levels, secret scanning, access control by scope, and packs framed as data.

### 7.2 First vertical slice

1. **Schema.** The DDL in §6.2 for `memory`, `memory_fts`, `memory_vec`, `memory_source`, `memory_use`, `context_pack`, `kg_entity`, `kg_edge`, `kg_edge_source`, `kg_predicate`, `board`, `card`, `card_event`, `channel`, `channel_member` and `chat_message`, with migrations.
2. **Memory service.** Write (validate, secret scan, hash dedup, embed, insert, link entities) and hybrid search (FTS5, brute force, entities, RRF). Embedder: fastembed with bge-small and bundled files. *Check:* a written memory can be found in under 10 ms; a duplicate is reinforced, not inserted.
3. **Rule extractor** over normalized events, producing `EDITED`, `TOUCHES`, `RAN_OK`/`RAN_FAILED`, `WORKED_ON`, `IMPLEMENTS` and `ABOUT` edges, with worktree-path normalization. *Check:* "who changed `src/x.ts` and for which task" is answered from the graph.
4. **MCP tools:** `codly_memory_search`, `codly_memory_write`, `codly_task_report`, `codly_board` (list, get, update, comment) and `codly_message` (send to the supervisor or user, read).
5. **Context broker.** The spawn pack and per-turn delta packs with a budget, the injection log, and the "facts, not commands" framing, for Claude and Codex first. *Check:* no memory is injected twice in a session; the token budget is never exceeded.
6. **Boards.** The shared kanban as a projection of `task` plus free cards, and per-agent boards that mirror Claude's TodoWrite and Task events and Codex plan updates.
7. **Chats.** 1:1 user ↔ agent chat with Hold, and the supervisor's DMs visible to the user.
8. **UI.** The memory browser (both search modes, provenance link into the timeline, pin, edit, archive, "Send to agent"), the review inbox, and a simple injection inspector.
9. **Office.** Library, filing-cabinet and whiteboard animations driven by real tool-call events.
10. **Jobs.** J1 dedup, J3 citation check and J5 task close. All deterministic.

### 7.3 Defer

- LLM extraction (W6) and reflection (J6), until the deterministic paths are measured.
- Semantic contradiction detection beyond functional predicates.
- The graph canvas with an as-of slider, communities and PageRank map (the slice ships only the entity page).
- Worker-to-worker DMs, which stay routed through the supervisor in v1.
- Importing Claude auto memory and Codex memories (W7).
- `sqlite-vec` or `usearch` ANN indexes, to revisit beyond about 250k vectors or once sqlite-vec's ANN is stable.
- Multilingual or "quality" embedding models (EmbeddingGemma, Qwen3) as options.
- Code-specific embeddings, which the code-retrieval report owns.
- Cross-machine sync and encryption at rest.

### 7.4 Risks to watch

- **`ort` is still a release candidate** (2.0.0-rc.15), pinned exactly by fastembed. Expect churn when 2.0 ships.
- **Agents may ignore tools.** That is why the pack carries the first turn. Measure the cited rate.
- **Review fatigue.** If too many proposals pile up, users stop reviewing. Tune the auto-accept policy per kind.
- **Quota.** Any LLM job draws on the user's plan. Keep them batched and skippable.

---

## 8. Unverified items and open questions

- **MD Pro internals** ("graph of agents, tickets and notes", the two labelled searches, "412 topics") come only from the Pro web page. The code isn't public.
- **bge-small latency on M-series chips** was not measured, because I downloaded no models. The §4.2 numbers are third-party MiniLM figures on an M4 Max. Measure bge-small and arctic-xs with Codly's real build.
- **Extraction with `--json-schema` and `--tools ""`** was verified by the sibling only for a small routing call (two turns, `stop_reason: tool_use`). Verify it on real task bundles.
- **The fastembed 7.5.0 builder names** (`with_cache_dir`, the user-defined model API) were not compiled. Check them against docs.rs.
- **Whether `autoMemoryDirectory` passed through `--settings` applies** to stream-json and `-p` worker sessions as the docs say ("any settings scope… or `--settings`"). Test it, and check what `system/memory_recall` events contain.
- **The exact Codex key to turn memories off for one run** (`-c features.memories=false`, or `memories.generate_memories=false`) needs testing. Memories are documented as unavailable in the EEA, the UK and Switzerland at launch.
- **Gemini CLI vs Antigravity CLI:** how memory behaves in each, given the docs' replacement notice.
- **All vendor benchmark numbers** (Mem0 2026, Zep's corrected LoCoMo, Copilot's A/B test, Letta's filesystem score) are self-reported. LoCoMo's answer key has disputed errors.
- **Generative Agents' scoring constants** (recency decay, the reflection threshold) were not re-verified today, so this report doesn't rely on them. MemoryOS's heat formula is used instead.
- **The local benchmarks use synthetic data** on an in-memory database. On-disk WAL databases with real text will be somewhat slower. The orders of magnitude, and the gotcha about the CTE form, are what matter.

---

## 9. Sources

**Munder Difflin and MemPalace**
- Repository (read locally): https://github.com/chaitanyagiri/munder-difflin, also referenced as https://github.com/HarnessMD/munder-difflin. Files: `HIVE.md`, `MEMORY_GRAPH_SPEC.md`, `docs/design/knowledge-graph.md`, `src/main/{memory,reflect,knowledge,palaceReap,db,hive}.ts`, `src/main/kg-core.cjs`, `src/renderer/src/components/{MemoryPanel,MemoryGraphPanel,TasksKanban,ThreadsPanel}.tsx`, `resources/skills/*`
- Site and Pro page: https://munderdiffl.in/ · https://munderdiffl.in/pro/ · blog posts: https://munderdiffl.in/blog/how-agents-remember-semantic-memory/ · https://munderdiffl.in/blog/keep-agent-semantic-memory-clean/
- MemPalace (read locally): https://github.com/MemPalace/mempalace (`mempalace/knowledge_graph.py`, `layers.py`, `embedding.py`, `general_extractor.py`, `mcp_server/*`, README benchmarks)
- Critique: Dey & Viradecha, "Spatial Metaphors for LLM Memory: A Critical Analysis of the MemPalace Architecture", https://arxiv.org/abs/2604.21284

**Memory systems**
- Mem0 paper: https://arxiv.org/abs/2504.19413 · 2026 algorithm: https://mem0.ai/blog/mem0-the-token-efficient-memory-algorithm
- Zep paper: https://arxiv.org/abs/2501.13956 (HTML: https://arxiv.org/html/2501.13956) · Graphiti: https://github.com/getzep/graphiti · deduplication: https://github.com/getzep/graphiti/blob/main/graphiti_core/utils/maintenance/dedup_helpers.py · edge prompt: https://github.com/getzep/graphiti/blob/main/graphiti_core/prompts/extract_edges.py · Zep on Mem0: https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/
- Letta: memory blocks https://docs.letta.com/guides/agents/memory-blocks · filesystem benchmark https://www.letta.com/blog/benchmarking-ai-agent-memory · Context Repositories https://www.letta.com/blog/context-repositories · sleep-time compute https://arxiv.org/abs/2504.13171
- LangMem: https://langchain-ai.github.io/langmem/concepts/conceptual_guide/ · LangGraph stores: https://docs.langchain.com/oss/python/langgraph/stores.md · langgraph-supervisor: https://github.com/langchain-ai/langgraph-supervisor-py
- Cognee: https://github.com/topoteretes/cognee · A-MEM: https://arxiv.org/abs/2502.12110 · MemoryOS: https://arxiv.org/abs/2506.06326
- ReasoningBank: https://arxiv.org/abs/2509.25140 · ACE: https://arxiv.org/abs/2510.04618
- GitHub Copilot Memory: https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/ · https://docs.github.com/en/copilot/concepts/agents/copilot-memory
- mempal (Rust): https://docs.rs/crate/mempal/0.9.1
- Survey: https://arxiv.org/abs/2603.07670 · MINJA: https://arxiv.org/abs/2503.03704

**Coding-agent CLIs and Anthropic guidance**
- Claude Code memory: https://code.claude.com/docs/en/memory · hooks: https://code.claude.com/docs/en/hooks · CLI reference: https://code.claude.com/docs/en/cli-reference · headless and bare mode: https://code.claude.com/docs/en/headless · agent teams: https://code.claude.com/docs/en/agent-teams
- Agent SDK usage on subscriptions: https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- Codex AGENTS.md: https://learn.chatgpt.com/docs/agent-configuration/agents-md · Codex memories: https://learn.chatgpt.com/docs/customization/memories?surface=app
- Gemini CLI memory: https://geminicli.com/docs/tools/memory/ · OpenCode rules: https://opencode.ai/docs/rules/
- Anthropic: context engineering https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents · memory tool https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool · multi-agent research https://www.anthropic.com/engineering/multi-agent-research-system
- Cognition: https://cognition.com/blog/dont-build-multi-agents

**Multi-agent coordination**
- Blackboard: https://arxiv.org/abs/2510.01285v2 · https://arxiv.org/html/2507.01701v1 · PatchBoard: https://arxiv.org/abs/2605.29313 · MetaGPT: https://arxiv.org/abs/2308.00352

**Benchmarks**
- LongMemEval: https://arxiv.org/abs/2410.10813 · https://github.com/xiaowu0162/LongMemEval
- LoCoMo: https://arxiv.org/abs/2402.17753 · audit: https://penfieldlabs.substack.com/p/we-audited-locomo-64-of-the-answer

**Knowledge graphs and storage**
- LazyGraphRAG: https://www.microsoft.com/en-us/research/blog/lazygraphrag-setting-a-new-standard-for-quality-and-cost/
- SQLite recursive CTEs: https://www.sqlite.org/lang_with.html · rusqlite bundled flags: https://raw.githubusercontent.com/rusqlite/rusqlite/master/libsqlite3-sys/build.rs
- Kuzu shutdown: https://www.theregister.com/2025/10/14/kuzudb_abandoned/ · SurrealDB licence: https://github.com/surrealdb/surrealdb/blob/main/LICENSE

**Embeddings and vector search**
- fastembed-rs: https://github.com/Anush008/fastembed-rs · features: https://docs.rs/crate/fastembed/7.5.0/features · model2vec-rs: https://github.com/MinishLab/model2vec-rs · model2vec results: https://github.com/MinishLab/model2vec/blob/main/results/README.md
- Rust embedding benchmark (M4 Max): https://github.com/jerrythomas/rust-embedding-bench
- Model cards: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2 · https://huggingface.co/BAAI/bge-small-en-v1.5 · https://huggingface.co/Snowflake/snowflake-arctic-embed-xs · https://huggingface.co/google/embeddinggemma-300m · https://huggingface.co/Qwen/Qwen3-Embedding-0.6B · https://huggingface.co/minishlab/potion-code-16M-v2 · https://huggingface.co/jinaai/jina-embeddings-v2-base-code · https://huggingface.co/nomic-ai/CodeRankEmbed
- sqlite-vec: https://github.com/asg017/sqlite-vec/releases · https://alexgarcia.xyz/sqlite-vec/rust.html · https://alexgarcia.xyz/sqlite-vec/features/vec0.html · hybrid search: https://alexgarcia.xyz/blog/2024/sqlite-vec-hybrid-search/index.html
- MCP Rust SDK: https://github.com/modelcontextprotocol/rust-sdk
- crates.io API (versions and licences queried 2026-10-25): https://crates.io/api/v1/crates/<name> for fastembed, ort, candle-*, model2vec-rs, sqlite-vec, rusqlite, usearch, hnsw_rs, instant-distance, tantivy, tokenizers, hf-hub, rmcp, kuzu, lbug, cozo, oxigraph, surrealdb, helix-db, lancedb, arroy, simsimd, petgraph, mempal, tiktoken-rs

**Local measurements (this report)**
- `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/vecbench/` (brute-force vectors) · `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/sqlbench.py` and `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/sqlbench2.py` (FTS5 and graph CTEs)
