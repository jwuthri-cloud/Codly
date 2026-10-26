# Context engineering & code retrieval for Codly

*Research report — 2026-10-25. Scope: verify the "Jev" leads, survey 2026 code-retrieval practice for coding agents, and specify what Codly's Rust core should build first, how to hand context to Claude Code / Codex / Gemini CLI / OpenCode, and how to measure it honestly.*

**Evidence labels used throughout**

| Label | Meaning |
|---|---|
| **[V-src]** | Verified by reading source code cloned today (path given) |
| **[V-doc]** | Verified in official docs / release notes fetched today |
| **[V-local]** | Verified by running something on this machine today (Apple M5 Pro, 18 cores, 48 GB, macOS 26.5) |
| **[V-crates]** | Version/metadata read from the crates.io API today |
| **[Claim]** | Vendor- or project-reported; not independently reproduced |
| **[Unverified]** | Secondary source only, or I could not confirm; treat with care |

---

## 0. TL;DR — decisions

1. **"Jev" is real**, but it is not a code-search library. It is **TypeSafe's hosted "System One" decision model** (launched Sept 2026): you POST `state` + typed questions (`noul` = yes/no probability, `choice`, `score`) and get calibrated probabilities back, $0.042 per M input tokens, output free [V-doc]. Everything the chatbot found (`BorisLeMeec/jev`, `nassim-arifette/jevgrep`, …) are **community wrappers that upload your source to that API** [V-src]. No local Jev exists; there are open look-alikes (Kev, Laya, von) [Claim].
2. **The token-saving claims are small-n and partly fragile.** Best evidence: `dzhng/jevgrep` (2.9k★) −25.8 % total cost on 10 SWE-bench tasks, single run each, and **~one task (pytest) carries most of the effect** (without it the saving is ≈ −3 %) [V-src, my arithmetic]. Independent attempts found effects within noise (`jev-axi`) or ~5 % uptake (`jev-skill-router`) [V-src].
3. **Do not put Jev on Codly's default path.** It contradicts local-first (source leaves the machine), and its per-file request model cannot scale to a 100k-file repo (≈83 min at the documented 1,200 rpm used by `jev`) [V-src]. Keep an *optional* `DecisionProvider` adapter (Jev via OpenRouter's `/api/alpha/decisions`, or a local Kev server) for reranking ≤ 20 candidates, off by default, with per-repo consent.
4. **What actually works in 2026 is hybrid, cheap, and lexical-first.** Claude Code, Codex and Gemini CLI ship **no code index** — agentic grep/glob/read (+ an Explore/Investigator subagent for context isolation) [V-doc/V-src]. Cursor reports semantic search on top of grep helps (+12.5 % offline accuracy, +2.6 % code retention on ≥1k-file repos) and built a local trigram regex index because `rg` takes >15 s on monorepos [V-doc, Claim]. ContextBench (Feb 2026) finds scaffolding sophistication barely matters and that agents over-explore [V-doc].
5. **Build first (vertical slice):** `ignore` walker → tree-sitter **tags** symbol table → **SQLite FTS5** (rusqlite 0.41 bundled → SQLite 3.54.1, FTS5 compiled in) over paths, symbols and 60-line chunks → **RRF** fusion → **agreement-based confidence** → pointer-style citations. No embeddings in v1. Measured locally on Linux (97k files) and a commit-replay pilot on `openai/codex` (Section 3): **AND-first FTS5 queries run in 1.5 ms p50 / 3.6 ms p95 on 536k chunks**; RRF localizes ≥1 changed file in the top 5 for **65 %** (subject-only queries) to **80 %** (full commit messages) of 276 commits, vs **34–38 %** for a naive-grep baseline.
6. **Deliver context both ways, asymmetrically:** a *tiny* pointer pack (≤ ~1.2k tokens: `path:lines — why — confidence`) in the task prompt, plus a **local stdio MCP sidecar** (`codly-mcp`) exposing 3–4 tools (`codly_search`, `codly_symbols`, `codly_explain_choice`, `codly_feedback`). Wire via `--mcp-config` (Claude Code), `-c mcp_servers.codly.*` (Codex), ACP `session/new.mcpServers` (Gemini CLI `--acp`, OpenCode `acp`), `OPENCODE_CONFIG_CONTENT` (OpenCode CLI).
7. **Ship the benchmark in the product.** A commit-replay localization benchmark (Section 6) runs locally in ~2.5 min for 300 commits on a 9k-file repo [V-local] and doubles as **per-repo self-calibration** of confidence thresholds. Token-savings claims require a separate paired agent study with ≥ 3 repeats per arm.

---

## 1. Verifying the user's leads

### 1.1 What "Jev" means

**Jev = TypeSafe AI's first "System One" model** — a decision model that "returns typed, probabilistic decisions instead of text" [V-doc: docs.typesafe.ai]. Not an acronym I could find; it is a model name. The `typesafe-ai` GitHub org (created 2024-05-28) publishes official Python/JS SDKs, skills, and an LLM-backed adapter [V-src: `gh api orgs/typesafe-ai`].

Verified API facts (docs.typesafe.ai/models and /primitives, fetched 2026-10-25) [V-doc]:

| Item | Value |
|---|---|
| Endpoint | `POST https://api.typesafe.ai/v1/systemone`, `Authorization: Bearer $TYPESAFE_API_KEY` |
| Models | `jev-1.14.0` (from 2026-10-14), `jev-1.13.0`; aliases `jev-latest`, `jev-preview` → 1.14 |
| Request | `{ "model", "state": string \| object \| array, "questions": { "<id>": { "type": "noul"\|"choice"\|"score", "instructions", "criteria"? } } }` |
| Response | `answers[id].noul` (P(yes)); `choice` + `probabilities` + `confidence`; `score` + `legend` + `probabilities` + `confidence`; `usage.input_tokens/output_tokens` |
| Limits | 64k tokens per request, 32k per (state + one question); `TypeSafe-Danger-Zone-Long-Context: true` → 262k ("slower and less accurate") |
| Price | $0.042 / M input tokens; output free |
| Rate limits | per org, tokens/s and requests/s, 429 on excess |
| Hosting | **Hosted only.** No self-hosted option documented. Text only for 1.13; 1.14 adds image `state` + `box` questions (via OpenRouter docs) |
| Data | "Jev is not trained on customer requests or responses"; ZDR for enterprise |

Also reachable through **OpenRouter** — relevant because Codly's API-key backend already uses OpenRouter: `POST https://openrouter.ai/api/alpha/decisions`, slugs `typesafe/jev-1.14`, `~typesafe/jev-latest`, `typesafe/jev-1.13`; the same page lists other "decision" models (`openai/gpt-6-luna-decisions`, `microsoft/microsoft-decision-1`, `liquid/d1-ultra`) [V-doc: openrouter.ai/docs/guides/community/jev.md; the alpha path returned 404 to an unauthenticated GET, as expected]. OpenRouter's `/api/v1/models` lists only `typesafe/jev-router` (a model/effort router built on Jev) [V-local: curl]. Also on Vercel AI Gateway as `typesafe-ai/jev` [V-src: jevgrep provider tables].

Wire shape as used by real clients (from `BorisLeMeec/jev/internal/typesafe/client.go`) [V-src]:

```json
// request
{ "model": "jev-latest",
  "state": { "goal": "where is the register flow",
             "file": { "path": "internal/app/api.go", "skeleton": "package app\nfunc (s *Server) signup(..." } },
  "questions": { "match": { "type": "noul",
      "instructions": "Is the file at `file.path` one of the places where what `goal` describes is actually carried out? ...",
      "criteria": { "true": "This file's own code performs it ...", "false": "Everything else ..." } } } }
// response
{ "model": "jev-1.14.0", "answers": { "match": { "type": "noul", "noul": 0.95 } },
  "usage": { "input_tokens": 812, "output_tokens": 0 } }
```

Vendor claims (not reproduced): 40–200× faster than frontier LLMs, "cannot hallucinate or produce type errors" [Claim, via eesel.ai/letsdatascience]. Community audits flag over-abstention (95 % "unknown" on ambiguous KoBBQ items when that option exists) and threshold drift under distribution shift [Claim, via awesome-typesafe-jev].

**Local look-alikes** (all [Claim], project-reported):
- `jaredpalmer/kev` (9.5k★, Apache-2.0): Kev-0.8B/4B/9B/27B on Qwen3.5/3.8, **TypeSafe-compatible API**, runs on Apple Silicon via MLX (Python `uv run python -m kev.serve`). Reported "new sources" accuracy dev: Kev-4B 0.817, Kev-27B 0.851, Jev 0.857.
- `receptron/laya` (MIT) + `1Panel-dev/laya-server`: Laya open-weights, Jev-compatible, ONNX Runtime, ~1.7 GB weights.
- `wfzyx/von` (Apache-2.0): "sub-15 ms, non-autoregressive, local drop-in alternative to TypeSafe Jev".
- Rust clients: `AbdelStark/typesafe-rs`, `AbdelStark/s1-rs`, `Twister915/typesafe-ai` (all created 2026-09-16, ≤15★) [V-src: gh api].

### 1.2 `github.com/BorisLeMeec/jev` — EXISTS

| | |
|---|---|
| What | Claude Code **plugin** + Go CLI `jev` (find / ask / scan / gain / probe) + **PreToolUse hook on `Read`** |
| Language / license | Go 1.22+, MIT; plugin v0.1.0; 42★; created 2026-09-17, single commit visible (`e81c1d0`) [V-src] |
| Backend | TypeSafe Jev API only (`internal/typesafe/client.go`, endpoint above, client-side pacing at 1,000 rpm, comment says service allows 1,200 rpm) |
| Code size | ~2.8k lines Go (`internal/run`, `internal/scan`, `internal/typesafe`, `internal/usage`) |

**How `jev find` works** [V-src: `internal/run/find.go`, `internal/scan/scan.go`]:
1. *Reduce* — walk tree (hard-coded skip dirs/exts), build a per-file **regex skeleton**: package/imports/declarations/routes/form fields (one generic `declRe` for all languages, ≤40 lines). README: 434 KB → 34 KB.
2. *Screen* — **one HTTP request per file** with a `noul` question over `{goal, file:{path, skeleton}}`; concurrency 8; default refuses > 800 files (`--max-files`). Code comment: packing several files into one request "measured far worse … the ranking inverts".
3. *Verify* — top 8 re-scored on full contents (truncated to `maxContent`).
4. *Locate* — top 3 chunked; a `choice` question picks the chunk → line number.
Output: `0.95  internal/app/api.go:81`; threshold `--min 0.6`.

**The Read hook** [V-src: `internal/run/hook.go`, `hooks/hooks.json`]: on `PreToolUse` for `Read` with no `offset/limit`, file ≥ 400 lines and ≤ 80 KB, it reads the **last user message from the transcript**, asks Jev which window matters, and returns `hookSpecificOutput.updatedInput` with a new `offset/limit` (~1/5 of the file) plus a note. Fails open on any error. This is a genuinely useful *technique* for Codly (Claude Code hooks can rewrite tool input — [V-doc: code.claude.com/docs/en/hooks]) independent of Jev.

**Claims vs evidence** [V-src: `bench/RESULTS.md`, `bench/tokenecon/TOKENS.md`]:

| Claim | Evidence | Assessment |
|---|---|---|
| P@1 0.96 vs BM25 0.33 vs grep 0.08 | 24 answerable queries + 6 negatives, 3 repos (1 used in development), **private repos, query sets unpublished**; labels by agents forbidden to run jev | Small n; the "grep" baseline is a naive alternation count, BM25 is file-level with the raw NL query; 12/24 queries were *constructed* so the key noun appears in zero files. No embedding baseline. Plausible directionally for vocabulary-gap queries, not a general rate. |
| −30 % billed tokens | 7 tasks × **1 run per arm**; median ratio 0.66; worst 1.02 | Mechanism is credible (fewer turns ⇒ fewer cache re-reads), but n=7 single-run; no variance estimate. |
| `ask` precision 1.00 | 9 questions / 217 per-file judgments | "no error observed", as the README itself says. |
| Read hook −41 % billed, 50/50 targets kept | 3 paired runs on Hugo (public, reproducible harness) + 52 synthetic targets | Best-documented part; still tiny n. |

README's own most useful datum: across 9 real sessions, **cache reads 422 M tokens vs all tool results 196 k** (0.04 %) — i.e. *what you put in context early is re-paid every turn* [Claim]. This drives Codly's "pointer pack, not content pack" decision (§5).

**Usable by Codly?** Only as a subprocess CLI (`jev find --json`) needing `TYPE_SAFE_AI_KEY`; no library/MCP. Not recommended: hosted-only, per-file fan-out, Go binary to bundle, 800-file default cap.

### 1.3 `github.com/nassim-arifette/jevgrep` — EXISTS

| | |
|---|---|
| What | CLI + **stdio MCP server** (`jevgrep mcp`, one tool `semantic_search_code`) that scores repository fragments with Jev and returns **verbatim excerpts with line numbers** under a token budget |
| Language / license | TypeScript, Node 24, MIT; npm `@nassim-arifette/jevgrep` v0.2.0; 105★; created 2026-09-20 [V-src] |
| Providers | TypeSafe direct (`jev-1.13.0` pinned), Vercel AI Gateway (`typesafe-ai/jev`), OpenRouter `/api/alpha/decisions` (`typesafe/jev-1.13`) |
| Notable engineering | per-repo authorization + explicit consent prompt before uploading source; secret-pattern exclusions; score cache keyed by (provider, model, query, fragment hash…); token-aware batching (≤64 questions, ≤256 KiB/request, 30 % tokenizer headroom); exit codes 0/2/3/4/130; JSON contract with coverage + stop reasons |
| MCP input schema | `{query: string, scope?: string[] (repo-relative), response_tokens?: number, allow_partial?: boolean}` [V-src: `src/mcp.ts`] |

Claims vs evidence: retrieval pilot on **3 small synthetic repos, 30 questions** (6 negatives): Recall@1 0.78, Recall@5 0.97, MRR 0.98, negatives 6/6 [V-src: `benchmarks/baselines/retrieval-vercel.json`]. **No baseline comparison**, development set (not held-out). README itself says the Codex/Claude Code MCP examples "have not yet been qualified with real … sessions" and the TypeSafe transport "has not been tested against a real account". There is **no persistent index** — every search re-inventories and re-sends uncached fragments.

**Usable by Codly?** Technically yes as an external MCP server (`jevgrep mcp --config <abs path>`), but same hosted-upload objection. Its consent/authorization UX and budgeted-excerpt contract are worth copying.

**Note:** the more significant project is **`dzhng/jevgrep`** (2,901★, MIT, `@dzhng/jevgrep` v0.8.5, CLI `jg`, created 2026-09-26) [V-src]. Hierarchical traversal (directories → previews → declarations, tree-sitter WASM for Python/Go/Rust, TS compiler for TS/JS) with Jev relevance judgments; outputs files + verbatim source to stdout. Its SWE-bench study (10 tuned Python tasks, `openai/gpt-5.6-sol` medium, **one sample per task**):

| Run | Solves | Cost |
|---|---|---|
| Saved no-Jev baseline | 8/10 | $7.62 |
| v0.3.x candidate (Sol only, Jev excluded) | 8/10 | $5.44 (−28.6 %) |
| v0.4.3 total incl. Jev | 8/10 | $5.66 = $4.18 Sol + $1.48 Jev (**−25.8 %**) |
| v0.5.0 | 8/10 | combined **+2–3 % vs v0.4.3**, "not a verified combined-cost improvement" (their words) |

**My check** [V-src arithmetic on `evals/results/total-cost-2026-09-28.md`]: `pytest-dev__pytest-6197` alone is $2.34 → $0.53. Excluding it, totals are $5.28 vs $5.13 → **−2.8 %**. The headline is dominated by one outlier; the authors' own caveats say as much.

### 1.4 "jevr" — no such single project

GitHub search returns only unrelated or tangential repos: `simxnherrera/jevr` and `mountainMath/JevR` (R clients for the System One API, 5★ each), `BillionsBobby/JevRouter` ("Jev-powered router for models, tools, and subagents", 467★), `chrisDeFouRire/jevrules`, `SamanPandey-in/jevrail`, `ajanm007/jevrag`, `hgqimo/JevRanker` [V-src: `gh search repos jevr`]. **Nothing named "jevr" is a code-retrieval tool.** The chatbot likely conflated these.

### 1.5 "repoctx" — generic name, ≥ 11 unrelated repos, none Jev-related

All have 0–1★ [V-src: `gh search repos repoctx`]. The two worth knowing:
- `mikolajmikolajczyk/repoctx` — **Rust**, **LGPL-3.0-or-later**, v0.15.3; tree-sitter 0.25 (+ ~20 grammar crates) → SQLite (`rusqlite 0.32 bundled`) at `.repoctx/index.db`; `definition`, callers/callees (petgraph), `search` = symbols + ripgrep matches; integrations for Claude/Codex/opencode [V-src]. Claims 97–99 % token reduction on helix — **versus "ripgrep + open every matched file"**, a strawman baseline [Claim]. Architecture is close to what Codly should build; license (LGPL) makes copying code awkward for a closed app; reading for ideas is fine.
- `harsha-moparthy/repoctx` — Python; claims "0.907 recall@16k on 50 PR-mined tasks vs 0.825 single-signal" for tree-sitter chunks + BM25 + dense + call-graph with a token-budget packer [Claim, 0★, not inspected further].

### 1.6 "TypeSafe Jev" for "cheap calibrated decisions" (which files, tools/skills, agent, need-more-context, human-review)

That description matches Jev's intended use (routing, gating, scoring). The ecosystem has tried each of those [V-src: awesome-typesafe-jev README, cloned]:

| Decision | Community attempt | What was measured |
|---|---|---|
| Which files to read | jev, jevgrep ×3, jev-graphify, Hermes JIT Context | §1.2–1.3; small-n positive results |
| Which skill | `shimo4228/jev-skill-router` (Claude Code UserPromptSubmit hook) | **28 of 539 suggestions (~5 %) followed in a week; author removed it** — Claude Code still shows every skill description, the model still chooses [V-src README] |
| Which model/agent | `jev-router`, `Codex Jev Router`, `JevRouter`, OpenRouter `typesafe/jev-router` | Mostly unmeasured or backtests of older policy [Claim] |
| Rank/guard tool calls | `shiftynick/jev-axi` | On a 390k-line repo "file ranking moved cost and file reads by less than run-to-run noise" (6 runs/condition; ±25–29 % swings); "the agent never loaded the skill unprompted" [V-src README] |
| Human review / done-gate | `jev-belay`, `Canny`, `jev-engineering` | Unmeasured beyond adversarial kits [Claim] |
| Independent eval | `rmax-ai/jevgrep-eval` (Codex "Luna" + native search vs jevgrep vs BM25) | Frozen corpus (22 admitted tasks) and harness only; **no live results published** — mock fixtures labelled as such [V-src] |

**Conclusion:** cheap decision models are a reasonable *component* for bounded classification (e.g., "is this candidate relevant?" over ≤ 20 candidates), but there is no evidence yet that they move end-to-end agent cost/quality on their own. Codly can get most of the same signals locally (Section 4.6: ranker agreement predicts hit-rate).

### 1.7 Can Codly use any of it from Rust?

| Path | How | Verdict |
|---|---|---|
| Jev via OpenRouter | `reqwest` POST `https://openrouter.ai/api/alpha/decisions` with the user's OpenRouter key; JSON as above | **OK as optional reranker/gate**, off by default, per-repo consent, ≤ 20 candidates/request |
| Jev direct | POST `api.typesafe.ai/v1/systemone`; or `typesafe-ai` Rust community crates | same, needs separate key |
| Kev / Laya local | spawn Kev's Python MLX server (TypeSafe-compatible) or run Laya ONNX via `ort` | heavy (Python/MLX or 1.7 GB ONNX); research-only for now |
| `jev` / `jg` / `jevgrep` CLIs | subprocess, parse `--json`/stdout | no — Node/Go runtime deps, upload source, no index |
| `jevgrep mcp` | register as an extra MCP server for the CLI agents | user's choice; Codly need not ship it |

Scale sanity check for per-file screening on a 100k-file repo: 100,000 requests ÷ 1,200 rpm ≈ **83 minutes** and ~1.1 GB × (skeleton ratio ≈ 8 %) ≈ 90 MB ≈ 22 M tokens ≈ $0.95 per *query* [my arithmetic from jev's own numbers]. Not viable as the primary path.

---

## 2. State of the art for agent code retrieval (2026)

### 2.1 What the major tools do

| Tool | Context strategy (2026) | Evidence |
|---|---|---|
| **Claude Code** | No index. Agentic `Glob`/`Grep`(ripgrep)/`Read`; CLAUDE.md loaded upfront; built-in **Explore** subagent (read-only; "keeps exploration results out of your main conversation context"; thoroughness quick/medium/very thorough); LSP tool; MCP tool search/deferral; MCP output warning at 10k tokens, cap 25k (`MAX_MCP_OUTPUT_TOKENS`) | [V-doc: code.claude.com sub-agents, mcp]; Anthropic: "CLAUDE.md files are naively dropped into context up front, while primitives like glob and grep allow it to navigate … just-in-time" (Sept 29 2025) [V-doc]; Boris Cherny: early versions used a local vector DB, agentic search "outperformed everything" — basis "mostly vibes" [Unverified, secondary reports] |
| **Codex CLI** | No index. Prompts: "prefer using `rg` or `rg --files` … much faster than alternatives like `grep`"; `file-search` crate = `ignore` + `nucleo` fuzzy matcher for @-mentions; AGENTS.md | [V-src: `codex-rs/core/gpt_5_codex_prompt.md`, `codex-rs/file-search/Cargo.toml`; no embedding index in `codex-rs` @ `970111f6`, 2026-10-23] |
| **Gemini CLI** | grep/glob/read tools; built-in `codebase_investigator` subagent (separate context loop, returns findings) | [V-doc: geminicli.com subagents (search result)]; v0.62.0 locally |
| **OpenCode** | grep/glob/read + LSP; ACP server (`opencode acp`) | [V-local: `opencode --help` v1.18.31] |
| **Cursor** | Semantic index (custom embedding model trained from agent traces, LLM-ranked targets) **plus** grep; local **sparse n-gram/trigram regex index** anchored to a git commit with a live overlay for edits | Semsearch (2025-11-06): +12.5 % avg accuracy (6.5–23.5 %) on internal Cursor Context Bench; A/B: +0.3 % code retention overall, **+2.6 % on ≥1,000-file codebases**, +2.2 % dissatisfied follow-ups without it; "the combination … leads to the best outcomes" [V-doc, Claim]. Fast-regex (2026-03-23): "`rg` invocations that take more than 15 seconds" in monorepos; index must be "very fresh" so the model reads its own writes [V-doc] |
| **Windsurf / Cognition** | **SWE-grep / SWE-grep-mini**: RL-trained retrieval subagent; ≤ 8 parallel tool calls/turn, ≤ 4 turns; tools grep/read/glob only; reward = mean of file- and line-level **F0.5** ("precision is prioritized over recall … polluting the main agent's context hurts more"); >2,800 tok/s (mini) / >650 tok/s | [V-doc: cognition.com/blog/swe-grep, 2025-10-16; Claim: results on internal CodeSearch Eval, "informal demo"] |
| **Augment** | Hosted semantic Context Engine; **Context Engine MCP** (local via Auggie CLI, or remote) since 2026-02-06 | "+80 % Claude Code + Opus 4.5" on 300 Elasticsearch PRs; metric undefined [V-doc, Claim] |
| **Sourcegraph Cody → Amp** | Replaced embeddings with Sourcegraph Search (keyword index) in Cody Enterprise; Amp "Librarian" subagent for remote code search | [Unverified secondary] |
| **GitHub Copilot** | Embedding model for VS Code context retrieval (Sept 2025): +37.6 % relative retrieval score, 8× smaller index | [Claim: github.blog] |
| **Zed** | Agent tools `grep`, `find_path`, `read_file`, `diagnostics`; no semantic index in docs | [Unverified, docs via search] |
| **Aider** | **Repo map**: tree-sitter tags → def/ref graph → personalized **PageRank** (networkx) → token-budgeted map (`--map-tokens`, default 1024; ×8 when no files in chat) | [V-src: `aider/repomap.py` @ 2026-05-22] — details §2.3.4 |

### 2.2 Published evidence: agentic search vs indexed retrieval

- **SWE-bench (2023)**, BM25 over files vs oracle: at 27k tokens BM25 recall Avg 44.4 %, *All* 39.8 %, *Any* 51.3 %; "in almost half of the instances … it retrieves none of the files" [V-src: arXiv 2310.06770 Table 3, extracted today]. Classic RAG with a *fixed* budget is weak.
- **ContextBench** (arXiv 2602.05892, 2026-02-05): 1,136 tasks / 66 repos / 8 languages, gold context at file/block/line. Agents: mini-SWE-agent, Agentless, SWE-agent, OpenHands, Prometheus. Findings: "sophisticated agent scaffolding yields only marginal gains in context retrieval" (mini-SWE-agent ≈ others: Pass@1 0.472 vs 0.452–0.512 with GPT-5); LLMs "consistently favor recall over precision"; big gap between explored and used context; block-level F1 < 0.45, line-level F1 < 0.35; Claude Sonnet 4.5 took 14.4 steps/instance, 29.7 lines/step, highest redundancy 0.708 [V-doc].
- **turbopuffer talk, AI Engineer Europe 2026** (ContextBench 50 tasks, Claude Code): capping reads to 50 lines raised file precision 65 → 81 %; adding semantic search 81 → 87 %; **recall did not improve** (49 → 39 %); semantic won 8 tasks, grep won 7 — "complementary" [V-doc talk page, Claim].
- **Localization research** (file level, SWE-bench Lite): Agentless ≈ 79.6 % Acc@5 (Claude 3.5, per LocAgent's table) [Unverified number]; **LocAgent** (graph-guided agent, fine-tuned Qwen2.5-Coder-32B) "up to 92.7 % accuracy on file-level localization", −86 % cost [V-doc abstract]; **CoSIL** top-1 43.3 % / 44.6 % (Lite / Verified) [V-doc abstract via search]; **SweRank** (ICLR 2026, retrieve-and-rerank, SweLoc dataset) beats agent-based systems with Claude 3.5 [V-doc abstract]; SweRank+ (2512.20482) multilingual/multi-turn. Numbers across papers conflict (LocAgent reported as 86.07 vs 73.36 in different papers) — trust only same-harness comparisons.
- **Embedding quality on code, cheap end:** `minishlab/potion-code-16M-v2` (static, 256-d, MIT) **CoIR avg 39.08 < BM25 42.31**; hybrid 43.36; teacher CodeRankEmbed (137M) 59.14 [V-doc: HF model card]. Static embeddings alone do not beat BM25 on code; a real transformer embedder does.

**Synthesis.** (1) Lexical search + agent iteration is a strong baseline that the best vendors still keep. (2) Indexes help mainly on large repos, vocabulary-gap queries, and *latency* (Cursor's regex index, Cognition's parallel RL subagent). (3) Precision matters more than recall for what enters the main context (Cognition's F0.5, ContextBench's "explored ≠ used"). (4) No one has strong public evidence that pre-injected RAG beats agentic search end-to-end; vendor numbers are internal. Codly should therefore **accelerate and sharpen agentic search** (fast tools, good first candidates, explicit confidence) rather than replace it.

### 2.3 Building blocks (Rust), versions verified on crates.io today [V-crates]

| Need | Crate (version, published) | Notes |
|---|---|---|
| Gitignore-aware parallel walk | `ignore` 0.4.33 (2026-08-04) | Same walker as ripgrep; `WalkBuilder::{hidden, max_filesize, threads, add_custom_ignore_filename, build_parallel}` [V-src] |
| Exact/regex search | `grep-searcher` 0.1.17, `grep-regex` 0.1.14, `grep-matcher` 0.1.9 (or facade `grep` 0.4.1) | ripgrep's library stack; Linux-kernel `rg -l -F` warm = **1.5–2.0 s** [V-local] — fine as a tool, too slow as the first step of every query |
| Full-text BM25 | `rusqlite` 0.41.0 (2026-10-24) `bundled` → **SQLite 3.54.1 with `-DSQLITE_ENABLE_FTS5`** | [V-src: `libsqlite3-sys` 0.39.1 `build.rs` lines 172–181] |
|  | `tantivy` 0.27.0 (2026-10-17) | Block-WAND top-k for OR queries (`query/boolean_query/block_wand_union.rs`) [V-src]; custom tokenizers; segment/merge model |
|  | `bm25` 2.3.2 | in-memory, fine for ≤ 10k docs |
| Parsing | `tree-sitter` 0.27.1 (2026-10-08), `tree-sitter-tags` 0.27.1 | Grammar crates export `LANGUAGE` + `TAGS_QUERY`: verified for rust 0.24.2, python 0.25.0, typescript 0.23.2 (`LANGUAGE_TYPESCRIPT`/`LANGUAGE_TSX`; its tags query is TS-only, combine with JS's) [V-src]; go 0.25.0, javascript 0.25.0, swift 0.7.4 versions verified, exports not checked |
|  | `tree-sitter-language-pack` 1.22.1 | 372 grammars, **default features download parsers at runtime + `libloading`** [V-src: Cargo.toml features `download`, `dynamic-loading`] — avoid in a signed/notarized macOS app, or build with `TSLP_LANGUAGES` static selection |
| Structural search | `ast-grep-core` / `ast-grep-language` 0.51.0 (2026-10-15) | Better exposed as a tool than used for indexing |
| LSP | `async-lsp` 0.2.4, `lsp-types` 0.97.0 (2024) | Optional precise refs later; `stack-graphs` is **archived** (2025-09) [V-src gh api] |
| Graph rank | `petgraph` 0.8.3 | PageRank à la Aider (implement power iteration; ~30 lines) |
| Watching | `notify` 8.2.1 (2026-10-25), `notify-debouncer-full` 0.7.0 | `new_debouncer(timeout, tick, handler)`, `debouncer.watch(path, RecursiveMode::Recursive)` [V-src] |
| Hashing | `blake3` 1.8.7, `xxhash-rust` 0.8.19 | content hash per file |
| Git | `gix` 0.90.0 (2026-10-22), `git2` 0.21.0 | read HEAD tree, diffs for incremental + benchmark replay |
| Tokens | `tiktoken-rs` 0.12.1 | or bytes/4 heuristic for budgeting (models differ anyway) |
| MCP server | `rmcp` 3.7.0 (2026-10-22, official SDK) | `#[tool_router]`, `#[tool]`, `#[tool_handler]`, `Parameters<T>`, `Json<T>`, `transport::stdio()` [V-src] |
| Embeddings (phase 2) | `fastembed` 7.5.0 (pins `ort =2.0.0-rc.15`): `JinaEmbeddingsV2BaseCode`, nomic v1.5, EmbeddingGemma300M, Snowflake Arctic; rerankers `BGERerankerBase`, `BGERerankerV2M3`, `JINARerankerV1TurboEn`, `JINARerankerV2BaseMultilingual`; `qwen3-text` feature | [V-src]. ONNX Runtime binary is a packaging cost |
|  | `model2vec-rs` 0.3.0 (MIT, pure Rust: safetensors + tokenizers) | static embeddings; see CoIR numbers above |
| Vector store | `sqlite-vec` 0.1.9 stable (0.1.10-alpha.4, last push 2026-05-18), `usearch` 2.26.6, `arroy` 0.9.0, `lancedb` 0.40.0 | sqlite-vec keeps one DB file but is pre-1.0 and quiet since May |

#### 2.3.1 SQLite FTS5 vs tantivy for Codly

| | SQLite FTS5 (via rusqlite bundled) | tantivy 0.27 |
|---|---|---|
| Ops | One file per repo; transactional with the rest of Codly's SQLite state; WAL; trivial backup | Separate index dir; commit/merge policy; reader reload |
| Top-k on OR queries | Scores **every** matching row (no WAND) → measured **51 ms p50 / 180 ms p95** on 536k chunks [V-local] | Block-WAND top-k [V-src] — expected much better on OR-heavy queries [not measured here] |
| AND/NEAR queries | **1.5 ms p50 / 3.6 ms p95** on 536k chunks [V-local] | fast |
| Tokenizer | built-ins (`unicode61`, `porter`, `trigram`); custom tokenizers need the C API (not wrapped by rusqlite) → **pre-split identifiers in Rust** and store terms | first-class custom tokenizers (camelCase/snake splitting) |
| Index size (Linux, 1.14 GB source) | 369 MB paths+symbols (regex-approx symbols, inflated) / **714 MB** with 536k contentless chunks [V-local] | not measured |
| Build cost (Python, single thread, Linux) | 44 s (paths+symbols) / 70 s (+chunks), RSS ≤ 150 MB [V-local] | — |

**Decision:** start with **FTS5 everywhere** and **AND-first query shaping** (AND → NEAR → OR-of-rarest fallback only if < k hits). Re-evaluate tantivy for the chunk index only if the in-product benchmark shows chunk p95 > 150 ms on real user repos.

#### 2.3.2 Tree-sitter symbol extraction

Use `tree-sitter-tags` with each grammar's `TAGS_QUERY` (Aider ships extra/adapted `*-tags.scm` for ~30 languages under `aider/queries/tree-sitter-language-pack/`, Apache-2.0 with upstream grammar licenses [V-src]). Tag captures give `@definition.{function,method,class,interface,module,macro}` and `@reference.{call,class,…}` — enough for a def/ref graph.

```rust
// Cargo: tree-sitter = "0.27.1", tree-sitter-tags = "0.27.1", tree-sitter-rust = "0.24.2"
// API checked against crate sources 2026-10-25; not compiled here.
use tree_sitter_tags::{TagsConfiguration, TagsContext};

pub struct SymbolRow { pub name: String, pub kind: String, pub is_def: bool,
                       pub start_line: u32, pub end_line: u32, pub start_byte: u32, pub end_byte: u32,
                       pub docs: Option<String> }

pub fn rust_config() -> anyhow::Result<TagsConfiguration> {
    Ok(TagsConfiguration::new(tree_sitter_rust::LANGUAGE.into(), tree_sitter_rust::TAGS_QUERY, "")?)
}

/// One TagsContext per worker thread (it owns a Parser + QueryCursor).
pub fn extract(ctx: &mut TagsContext, cfg: &TagsConfiguration, src: &[u8]) -> anyhow::Result<Vec<SymbolRow>> {
    // `tag.span` is the NAME node's position (tags.rs:508); `tag.range` is the whole item's bytes.
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(src.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1)).collect();
    let line_of = |byte: usize| line_starts.partition_point(|&s| s <= byte) as u32; // 1-based
    let (tags, _had_parse_error) = ctx.generate_tags(cfg, src, None)?;
    let mut out = Vec::new();
    for tag in tags {
        let tag = tag?;
        out.push(SymbolRow {
            name: String::from_utf8_lossy(&src[tag.name_range.clone()]).into_owned(),
            kind: cfg.syntax_type_name(tag.syntax_type_id).to_string(),
            is_def: tag.is_definition,
            start_line: line_of(tag.range.start),
            end_line: line_of(tag.range.end.saturating_sub(1)),
            start_byte: tag.range.start as u32, end_byte: tag.range.end as u32,
            docs: tag.docs,
        });
    }
    Ok(out)
}
```

Chunking: prefer **symbol-aligned chunks** (definition `range` → line span, split if > 120 lines, merge tiny neighbours) with a 60-line window fallback for unparsed files; store chunk ↔ symbol id.

#### 2.3.3 Hybrid fusion (RRF) and query rewriting

Reciprocal Rank Fusion: `score(d) = Σ_r 1/(k + rank_r(d))`, k = 60. It needs no score normalization and is robust when rankers' scores are incomparable (BM25 over symbols vs chunks vs paths) — exactly Codly's case.

```rust
pub fn rrf<'a>(lists: &[&[(&'a str, f32)]], k: f32) -> Vec<(&'a str, f32)> {
    let mut acc: std::collections::HashMap<&str, f32> = Default::default();
    for list in lists {
        for (rank, (id, _)) in list.iter().enumerate() {
            *acc.entry(id).or_default() += 1.0 / (k + rank as f32 + 1.0);
        }
    }
    let mut v: Vec<_> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}
```

Query rewriting, cheapest first: (1) identifier splitting + stopwords (done in pilot); (2) expand with symbols that appear in the user's *open/mentioned* files; (3) let the calling agent rewrite — it is already an LLM, so expose `codly_search(query, terms?)` and let it pass identifiers it guesses; (4) optional HyDE-style rewrite with a small model via the API backend (phase 2, measure first).

#### 2.3.4 Aider-style repo map (phase 1.5)

From `aider/repomap.py` [V-src]: nodes = files; edge referencer→definer per identifier with weight `mul × sqrt(num_refs)`, where `mul` ×10 if the identifier is mentioned in the chat, ×10 if snake/kebab/camel and length ≥ 8, ×0.1 if it starts with `_`, ×0.1 if defined in > 5 files, and ×50 when the referencer is a file already in chat; personalization `100/len(fnames)` for chat files / mentioned files / path components matching mentioned identifiers; `nx.pagerank(G, weight="weight", personalization=…, dangling=…)`; then binary search over number of tags to fit `max_map_tokens` (±15 %). Codly can reproduce this over its `refs` table and use the personalized rank as **a fourth ranker in RRF** and to render an "orientation map" for new sessions.

#### 2.3.5 Reranking options (phase 2+)

| Option | Cost | Note |
|---|---|---|
| Cross-encoder (fastembed `BGERerankerBase`/`JINARerankerV1TurboEn`, ONNX) | ~tens of ms per 20–50 pairs on CPU [not measured] | adds ONNX Runtime |
| Decision model (Jev via OpenRouter, Kev local) | one request scores ≤ 20 candidates (`noul` per candidate) | remote upload / heavy local |
| The calling agent | free in Codly terms | Give it 10 pointers + confidence; it reads 2–3. This is what the evidence supports today |

---

## 3. Local measurements (this machine, today)

### 3.1 Scale test: Linux kernel, 97k files [V-local]

Corpus: `torvalds/linux` shallow clone (97,203 tracked files; 96,887 indexed; 1.14 GB text). Script: `scratchpad/scripts/fts5_scale.py` (Python 3.12 stdlib, `sqlite3` 3.51.0 — not the Rust stack; numbers are an upper bound for the Rust implementation's single-thread cost).

| Step | Result |
|---|---|
| `rg --files` walk | 97,083 paths in **0.14–0.15 s** |
| FTS5 paths + symbols (regex-approximated, 3.9 M symbol strings — over-counted vs tree-sitter) | build **43.7 s**, optimize 0.9 s, DB **369 MB**, peak RSS **140 MB** |
| + 536,631 contentless 80-line chunks | build **69.6 s**, optimize 3.8 s, DB **714 MB**, peak RSS **145 MB** |
| File/symbol BM25 query (OR of terms, top 20) | **2.8 ms p50 / 8.4 ms p95** |
| Chunk BM25, OR of all terms | 51 ms p50 / 180 ms p95 |
| Chunk BM25, **AND** of terms | **1.5 ms p50 / 3.6 ms p95** |
| Chunk BM25, NEAR(…, 20) | 1.5 ms p50 / 3.1 ms p95 |
| Chunk BM25, OR of 3 rarest terms | 46.6 ms p50 / 163 ms p95 (OR cost is driven by scoring all matches, not term count) |
| `rg -l -F <literal>` warm | **1.5–2.0 s** per query |
| Sample: "tcp congestion control" → | `net/ipv4/tcp_cong.c`, `net/ipv4/sysctl_net_ipv4.c`, `net/ipv4/bpf_tcp_ca.c`, … |

### 3.2 Commit-replay localization pilot: `openai/codex` [V-local]

Repo: `openai/codex` (9,248 files at base; history since 2026-09-01). Last **300 first-parent commits**, index replayed to each commit's **exact parent** (base snapshot + incremental application of every diff — same mechanism Codly's incremental indexer needs). Gold = Modified/Deleted/Renamed files that existed at the parent (lockfiles/snapshots/binaries excluded); 22 commits with > 25 gold files and 2 with no retrievable gold skipped → **276 evaluated**, median 5 gold files. Script: `scratchpad/scripts/commit_bench.py`. Base index 9.8 s; eval 72 ms/commit; replay update 292 ms/commit (Python).

| Method | Query = subject only: Hit@1 | Hit@5 | R@10 | MRR | Query = full message: Hit@5 | R@10 | MRR |
|---|---|---|---|---|---|---|---|
| naive grep (files containing most query words) | 0.11 | 0.38 | 0.20 | 0.24 | 0.34 | 0.17 | 0.21 |
| BM25 paths only | 0.12 | 0.24 | 0.12 | 0.19 | 0.24 | 0.11 | 0.19 |
| BM25 symbols + paths | 0.30 | 0.59 | 0.29 | 0.43 | 0.71 | 0.37 | 0.55 |
| BM25 60-line chunks (max chunk per file) | 0.33 | 0.62 | 0.35 | 0.46 | 0.75 | 0.46 | 0.59 |
| **RRF(symbols, chunks)** | **0.37** | **0.65** | **0.36** | **0.50** | **0.80** | **0.46** | **0.64** |

Context size (subject-only): top-10-chunk content pack median **9.9k tokens** (p90 11.1k) vs reading the top-5 RRF files whole **33k** vs all gold files whole **37k**.

**Confidence signal — agreement between independent rankers** (|top-5 symbols ∩ top-5 chunks|), subject-only queries:

| agree | n | RRF Hit@5 | Hit@1 |
|---|---|---|---|
| 0 | 88 | 0.49 | 0.19 |
| 1 | 115 | 0.65 | 0.40 |
| 2 | 53 | **0.85** | 0.53 |
| 3 | 19 | 0.79 | 0.58 |

Gate "agree ≥ 2" covers 26 % of queries at Hit@5 0.84 (full messages: 35 % coverage at 0.89). Top-1 chunk-score terciles: Hit@5 0.47 / 0.71 / 0.77. The **score-gap margin was not predictive** (flat 0.63/0.63/0.69). So: *agreement + absolute top score* are usable confidence features; margins are not.

**Caveats (read before quoting):** one repo, one language family (Rust/TS), regex symbols (not tree-sitter), commit messages are not user prompts (full messages leak identifiers; subject-only is closer to real tasks), file-level only, no embeddings or graph ranker yet. This is a pilot of the *method*, not a product claim.

---

## 4. Recommended Rust implementation (vertical slice)

### 4.1 Shape

```
            ┌──────────── codly-core (Rust, in Tauri app) ─────────────┐
 notify ──▶ │ watcher ─▶ change queue ─▶ indexer workers (rayon)       │
 git HEAD ─▶│                │  parse(tree-sitter tags) → chunks        │
            │                ▼                                          │
            │   SQLite (per repo, WAL):  files │ symbols │ refs │       │
            │                fts_files │ fts_chunks │ file_rank          │
            │                ▲                                          │
            │ retrieval: parse query → [paths, symbols, chunks, (rank)] │
            │            → RRF → confidence → citations → pack/budget   │
            │                ▲              │                           │
            └────────────────┼──────────────┼───────────────────────────┘
                 UDS (per-session token)    └─▶ events → Workspace timeline + Living Agent World
                             │
              codly-mcp (tiny stdio sidecar, rmcp) ◀── Claude Code / Codex / Gemini / OpenCode
```

### 4.2 Dependencies (pin exactly; all versions read from crates.io 2026-10-25)

```toml
[dependencies]
ignore = "0.4.33"
grep-searcher = "0.1.17"
grep-regex = "0.1.14"
grep-matcher = "0.1.9"
rusqlite = { version = "0.41.0", features = ["bundled", "functions", "hooks"] } # SQLite 3.54.1 + FTS5
tree-sitter = "0.27.1"
tree-sitter-tags = "0.27.1"
tree-sitter-rust = "0.24.2"
tree-sitter-typescript = "0.23.2"
tree-sitter-javascript = "0.25.0"
tree-sitter-python = "0.25.0"
tree-sitter-go = "0.25.0"
tree-sitter-swift = "0.7.4"
notify = "8.2.1"
notify-debouncer-full = "0.7.0"
blake3 = "1.8.7"
rayon = "1.12.0"
petgraph = "0.8.3"
gix = { version = "0.90.0", default-features = false }   # HEAD tree + diffs; feature set TBD
rmcp = { version = "3.7.0", features = ["server", "macros", "transport-io", "schemars"] } # sidecar crate only
# phase 2, behind cargo features, only if the benchmark says so:
# tantivy = "0.27.0"
# fastembed = "7.5.0"        # pulls ort = "=2.0.0-rc.15"
# model2vec-rs = "0.3.0"
# sqlite-vec = "0.1.9"
```

### 4.3 Storage schema (one DB per repo at `~/Library/Application Support/Codly/index/<repo-id>.sqlite`)

```sql
PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;

CREATE TABLE files(
  id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, lang TEXT, bytes INTEGER, lines INTEGER,
  mtime_ns INTEGER, inode INTEGER, blake3 BLOB, git_blob TEXT, parse_ok INTEGER, indexed_at INTEGER);
CREATE TABLE symbols(
  id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  name TEXT NOT NULL, kind TEXT, is_def INTEGER, start_line INTEGER, end_line INTEGER,
  start_byte INTEGER, end_byte INTEGER, parent_id INTEGER, docs TEXT);
CREATE INDEX symbols_name ON symbols(name);
CREATE TABLE refs(file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE, name TEXT NOT NULL, line INTEGER);
CREATE INDEX refs_name ON refs(name);
CREATE TABLE chunks(
  id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  start_line INTEGER, end_line INTEGER, symbol_id INTEGER, tokens INTEGER);
-- rowid = files.id ; *_terms = identifier-split, lowercased tokens produced in Rust
CREATE VIRTUAL TABLE fts_files USING fts5(path_terms, symbol_terms, doc_terms,
  tokenize='unicode61 remove_diacritics 2', prefix='2 3');
-- rowid = chunks.id ; contentless: source text is re-read from disk (or blob) for snippets
CREATE VIRTUAL TABLE fts_chunks USING fts5(body_terms, content='', contentless_delete=1,
  tokenize='unicode61 remove_diacritics 2');
CREATE TABLE file_rank(file_id INTEGER PRIMARY KEY, pagerank REAL);
CREATE TABLE retrieval_log(id INTEGER PRIMARY KEY, session_id TEXT, ts INTEGER, query TEXT,
  result_json TEXT, confidence TEXT, used_json TEXT);   -- feeds calibration + explainability
```

Identifier splitting (done in Rust before insert): `retainedSampling` → `retainedsampling retained sampling`; `io_uring_submit` → `io_uring_submit io uring submit`. `unicode61` already splits on `_`, but not camelCase.

### 4.4 Indexing & incremental updates

- **Cold index:** `ignore::WalkBuilder` (`.max_filesize(Some(1 << 20))`, `.add_custom_ignore_filename(".codlyignore")`, git ignores on) → bounded channel → `rayon` pool parses (thread-local `TagsContext`) → **single writer thread** batching 500 files/transaction. Skip binaries (NUL in first 8 KiB), minified (> 500-char average line), generated (`@generated`, `DO NOT EDIT` headers), vendored dirs.
- **Change detection:** `notify-debouncer-full` (200–500 ms debounce) on the repo root *and each Codly worktree*; for each event path compare (`mtime_ns`, size, inode) → re-hash with blake3 only if changed → reparse that file only, `DELETE` + re-`INSERT` its rows in one transaction. On startup and on `git checkout`/`rebase`/branch switch (watch `.git/HEAD` and refs), diff the stored `git_blob` ids against `gix` HEAD tree to resync without re-reading unchanged files.
- **Worktrees:** agents edit in separate git worktrees. Index the main checkout once; for each worktree keep a **delta overlay** (rows for files whose blob differs from base, plus tombstones), and query `base ∪ overlay − tombstones`. This mirrors Cursor's "index anchored to a commit + layer on top" [V-doc] and keeps per-agent views consistent with what *that* agent has written ("read your own writes").
- **Freshness contract:** a retrieval call first drains pending change events for its scope (bounded wait ≤ 100 ms) and stamps results with `index_seq`; citations to files modified after `index_seq` are re-validated (line ranges re-mapped or dropped).

### 4.5 Query pipeline

1. **Parse:** detect identifiers/paths/globs in the query (`foo::bar`, `fooBar`, `src/x/*.rs`) → exact symbol lookup (`symbols.name = ?`) and path filter first. Exact hit ⇒ return immediately with confidence `exact`.
2. **Candidates** (each top-50): (a) `fts_files` with `bm25(fts_files, 1.0, 4.0, 2.0)` over path/symbol/doc terms; (b) `fts_chunks` with **AND → NEAR(…, 20) → OR-of-rarest-4** fallback until ≥ 20 hits; (c) personalized PageRank over def/ref graph seeded by mentioned files/identifiers (phase 1.5); (d) optional `grep-searcher` literal pass when the query has a quoted string or a rare identifier.
3. **Fuse:** RRF (k = 60) at file level; keep each file's best 1–3 chunks.
4. **Confidence** (§4.6) and **citations** (§4.7); **budget** (§4.8).

```sql
-- (b) chunk query, AND form; '?1' = '"retained" AND "sampling" AND "guardian"'
SELECT c.id, c.file_id, c.start_line, c.end_line, bm25(fts_chunks) AS s
FROM fts_chunks JOIN chunks c ON c.id = fts_chunks.rowid
WHERE fts_chunks MATCH ?1 ORDER BY s LIMIT 50;
```

### 4.6 Confidence & fallback policy

Features (cheap, local): `agree5` = |top-5(files ranker) ∩ top-5(chunk ranker)|; `top1` = best chunk BM25 (normalize per repo by percentile over the benchmark run); `matched_terms/query_terms` of the best chunk; exact-symbol hit; number of distinct directories in top-5 (dispersion).

Initial policy (fit on the codex pilot; **re-fit per repo** by the built-in benchmark, §6):

| Level | Rule | Pilot Hit@5 | What Codly does |
|---|---|---|---|
| `exact` | exact symbol/path match | — | return definition span + top refs |
| `high` | `agree5 ≥ 2` **and** `top1 ≥ p66` | ≈ 0.85 | pointer pack with 3–5 citations + snippet of the #1 span |
| `medium` | `agree5 = 1` or `top1 ≥ p33` | ≈ 0.65–0.70 | pointer pack with ≤ 8 citations, **no snippets**, plus "verify by reading" |
| `low` | otherwise | ≈ 0.49 | **"Not confident — search yourself."** Return only suggested `rg` patterns (rarest query terms, split identifiers, likely dirs) and *no* file list in the prompt pack |

Rule of thumb from Cognition and ContextBench: **when unsure, inject less** — a wrong pack costs more than no pack.

### 4.7 Citation format

Prompt form (one line each, ~30–40 tokens):

```
[C1] codex-rs/core/src/guardian/transport.rs:120-178  fn retain_sampling  — symbol+body match "retained sampling"; ranks sym#1 chunk#2  (high)
[C2] codex-rs/core/src/guardian/mod.rs:40-71  struct GuardianTransport  — referenced by C1; def/ref neighbour  (medium)
```

Structured form (MCP `structuredContent` / Codly event payload):

```json
{ "query": "retained sampling in guardian transport", "index_seq": 18423, "confidence": "high",
  "citations": [{ "id": "C1", "path": "codex-rs/core/src/guardian/transport.rs",
     "lines": [120, 178], "symbol": {"name": "retain_sampling", "kind": "function"},
     "why": ["symbol_terms matched: retained, sampling", "chunk bm25 rank 2"],
     "rankers": {"symbols": 1, "chunks": 2, "graph": null}, "rrf": 0.0325,
     "tokens": 610, "blob": "9f2c…" }],
  "fallback": { "rg": ["retain_sampling", "RetainedSampling", "guardian.*sampl"] } }
```

`why` is mechanical evidence, never prose generated by a model — this is what `codly_explain_choice` shows in the "click an agent → why these files" panel.

### 4.8 Token budgeting, caching, isolation

- **Pointer pack by default** (≤ 1,200 tokens: ≤ 10 citations + 1-line repo orientation). Snippet expansion only at `high` confidence and ≤ 3,000 tokens total. Hard cap 8,000. Pilot: a 10-chunk *content* pack is ~10k tokens; a 10-citation pointer pack is ~0.4–0.5k (≈ 20× smaller). Since context is re-sent every turn (cache reads dominate billed tokens, per jev's 9-session measurement), every 1k injected tokens costs ≈ 1k × remaining turns.
- Put the pack in the **task prompt (first user message)**; put static, task-independent tool instructions in a cacheable place (`--append-system-prompt-file` for Claude Code, `AGENTS.md`-style for others).
- **Per-session cache:** remember which citations/spans were already returned to a session; subsequent tool calls return `"already_returned": ["C1"]` instead of re-sending text. Cache query results by `(scope, normalized query, index_seq)`.
- **Context isolation per subagent:** each spawned agent gets its own `codly-mcp` instance (args `--session <id> --worktree <path>`), its own budget ledger, its own scope (worktree overlay), and its own `retrieval_log` rows. Never share one agent's retrieved snippets with another agent implicitly; the orchestrator passes **citations** (pointers), not contents, between agents.
- **Explainability & Living Agent World:** emit real events only — `retrieval.requested {session, query}`, `retrieval.completed {session, confidence, citations[]}`, plus the agent's own `Grep/Glob/Read` tool events parsed from its stream (`stream-json` / `--json`). The pixel office can show an agent "at the library" exactly when one of these events is in flight.

### 4.9 Targets for a 100k-file repo (Apple Silicon)

Grounded in §3.1 (Python single-thread upper bound) — Rust targets, not yet measured:

| Metric | Target |
|---|---|
| Cold full index (tags + chunks), 18 cores | ≤ 60 s wall (Python single-thread did 70 s without tree-sitter) |
| Incremental single-file update | ≤ 50 ms p95 end-to-end after debounce |
| Branch switch resync (1k changed files) | ≤ 5 s |
| Symbol/path query | ≤ 10 ms p95 (measured 8.4 ms) |
| Hybrid query (AND-first) end-to-end incl. RRF + citations | ≤ 50 ms p95; hard timeout 300 ms then degrade to files-only |
| DB size | ≤ 0.8× source bytes (measured 0.63× with approximate symbols + contentless chunks) |
| RSS | ≤ 300 MB while indexing, ≤ 100 MB idle (measured Python peak 145 MB) |
| Indexer CPU | `nice` background; pause when on battery < 20 % |

---

## 5. Delivering context to CLI agents

### 5.1 Options

| | (a) Prepend context pack | (b) Local MCP server tools | (c) Both |
|---|---|---|---|
| Works with every CLI | yes (it's text) | Claude Code, Codex, Gemini CLI, OpenCode, ACP agents: yes | yes |
| Agent must choose to use it | no | **yes** — evidence says agents often don't (jev-axi: never loaded unprompted; turbopuffer: Claude Code grep-oriented) | pack makes the tools discoverable |
| Token cost | paid every turn | only when called; tool schemas cost a little (Claude Code defers MCP tools behind tool search) | small pack + on-demand |
| Freshness | stale after first edit | live (`index_seq`) | live for follow-ups |
| Explainability | static | every call logged with evidence | best |

**Recommendation: (c) both, asymmetric.** Pointer pack (≤ 1.2k tokens, confidence-gated, omitted at `low`) + `codly-mcp` with ≤ 4 tools. The pack seeds the first turn (removing 1–3 search turns is where the measured savings come from); the tools serve follow-ups with fresh, budgeted answers.

### 5.2 Tools

| Tool | Input | Output |
|---|---|---|
| `codly_search` | `{query: string, scope?: string[], terms?: string[], k?: int ≤ 20, budget_tokens?: int ≤ 4000, include_snippets?: bool}` | citations + confidence + `fallback.rg` |
| `codly_symbols` | `{name?: string, path?: string, kind?: string, refs?: bool}` | definitions (path:lines, signature line) and optionally reference sites |
| `codly_explain_choice` | `{citation_ids?: string[]} ` | ranker ranks, matched terms, graph edges, confidence features for the last results |
| `codly_feedback` | `{used: string[], unhelpful?: string[]}` | ack — gives Codly implicit relevance labels for calibration |

Tool descriptions must say *when* to use them (vocabulary unknown, cross-cutting feature) and *when not* (known exact symbol → use grep/LSP), mirroring jev's SKILL.md guidance. Keep tool output ≤ 10k tokens (Claude Code warns above that; default cap 25k) [V-doc].

```rust
// codly-mcp sidecar; rmcp 3.7.0 API checked against crate tests 2026-10-25 (not compiled here)
use rmcp::{ServerHandler, ServiceExt, Json, tool, tool_handler, tool_router,
           handler::server::{router::tool::ToolRouter, wrapper::Parameters}};
use schemars::JsonSchema; use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs { pub query: String, #[serde(default)] pub scope: Vec<String>,
                        #[serde(default)] pub terms: Vec<String>, pub k: Option<u32>, pub budget_tokens: Option<u32> }
#[derive(Serialize, JsonSchema)]
pub struct SearchResult { pub confidence: String, pub citations: Vec<Citation>, pub fallback_rg: Vec<String> }
#[derive(Serialize, JsonSchema)]
pub struct Citation { pub id: String, pub path: String, pub lines: [u32; 2], pub why: Vec<String> }

#[derive(Clone)]
pub struct CodlyMcp { core: CoreClient /* UDS client with session token */, tool_router: ToolRouter<Self> }

#[tool_router]
impl CodlyMcp {
    pub fn new(core: CoreClient) -> Self { Self { core, tool_router: Self::tool_router() } }

    #[tool(description = "Find code by behaviour or concept when you don't know the identifier. Returns path:line citations with a confidence level. If confidence is 'low', grep with the suggested patterns instead.")]
    async fn codly_search(&self, Parameters(a): Parameters<SearchArgs>) -> Result<Json<SearchResult>, String> {
        self.core.search(a).await.map(Json).map_err(|e| e.to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CodlyMcp {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let core = CoreClient::connect_from_args()?;          // --session, --worktree, token via env
    let svc = CodlyMcp::new(core).serve(rmcp::transport::stdio()).await?;
    svc.waiting().await?;
    Ok(())
}
```

**Process model:** ship `codly-mcp` as a Tauri sidecar (`externalBin`), stdio transport (the only transport ACP agents **must** support [V-doc: ACP session-setup]); it proxies to the running core over a Unix domain socket in the app's support dir, authenticated by a per-session token passed via env. Avoid a localhost HTTP MCP port (port conflicts, other local processes can call it).

### 5.3 Exact wiring per CLI

**Claude Code** (v2.1.309 locally) [V-doc + V-local `claude --help`]:

```bash
claude -p "$TASK_WITH_POINTER_PACK" \
  --output-format stream-json --verbose \
  --mcp-config '{"mcpServers":{"codly":{"command":"/Applications/Codly.app/Contents/MacOS/codly-mcp","args":["--session","S1","--worktree","/path/wt"],"env":{"CODLY_TOKEN":"…"}}}}' \
  --append-system-prompt-file "$CODLY_RULES_MD"
# --strict-mcp-config only for benchmark isolation (it drops the user's own MCP servers).
# Optional observer hooks via --settings '{"hooks":{"PostToolUse":[{"matcher":"Grep|Glob|Read","hooks":[{"type":"command","command":"…/codly-hook"}]}]}}'
```

Notes: `--mcp-config` with `-p` waits for pending servers up to `MCP_TIMEOUT` (30 s default); hook `additionalContext` strings are capped at 10,000 chars [V-doc]. A PreToolUse hook *can* rewrite `Read` input (`hookSpecificOutput.updatedInput`) — Codly should **not** rewrite reads in v1; instead return `additionalContext` like "Codly: lines 120–178 most relevant (high); file is 1,900 lines" (non-destructive version of jev's hook).

**Codex CLI** (0.161.0 locally) — `-c` values are parsed as TOML [V-local `codex --help`]; fields from `codex-rs/config/src/mcp_types.rs` [V-src]: `command, args, env, env_vars, startup_timeout_sec, tool_timeout_sec, enabled, required, enabled_tools, disabled_tools, supports_parallel_tool_calls`:

```bash
codex exec --json -C /path/wt \
  -c 'mcp_servers.codly.command="/Applications/Codly.app/Contents/MacOS/codly-mcp"' \
  -c 'mcp_servers.codly.args=["--session","S2","--worktree","/path/wt"]' \
  -c 'mcp_servers.codly.tool_timeout_sec=30' \
  -c 'mcp_servers.codly.supports_parallel_tool_calls=true' \
  "$TASK_WITH_POINTER_PACK"
# usage arrives as {"type":"turn.completed","usage":{"input_tokens","cached_input_tokens",
#   "cache_write_input_tokens","output_tokens","reasoning_output_tokens"}}  [V-src exec_events.rs]
```

**Gemini CLI** (0.62.0 locally): no flag to add a server ad hoc — only `--allowed-mcp-server-names` (filter) [V-local]. Use **ACP** (`gemini --acp`) and pass the server in `session/new`; or write `.gemini/settings.json` (`mcpServers.codly = {command, args, env, cwd, timeout, trust:false, includeTools}`) inside the Codly-owned worktree [V-doc].

**OpenCode** (1.18.31): `opencode acp` (ACP) or `opencode run` with `OPENCODE_CONFIG_CONTENT` (env var read by the binary [V-local: strings]):

```bash
OPENCODE_CONFIG_CONTENT='{"$schema":"https://opencode.ai/config.json","mcp":{"codly":{"type":"local","command":["/Applications/Codly.app/Contents/MacOS/codly-mcp","--session","S3"],"enabled":true}}}' \
  opencode run "$TASK_WITH_POINTER_PACK"
# "mcp" object shape per OpenCode docs [Unverified today — check docs before shipping]
```

**ACP agents** (Gemini `--acp`, `opencode acp`, Claude/Codex/Pi via ACP adapters) — `session/new` [V-doc agentclientprotocol.com]:

```json
{"jsonrpc":"2.0","id":1,"method":"session/new","params":{
  "cwd":"/abs/path/wt",
  "mcpServers":[{"name":"codly","command":"/Applications/Codly.app/Contents/MacOS/codly-mcp",
                 "args":["--session","S4"],"env":[{"name":"CODLY_TOKEN","value":"…"}]}]}}
```

Cursor CLI, Copilot CLI and Pi were not installed here; their MCP wiring is **[Unverified]** — verify per CLI before claiming support.

---

## 6. Benchmark Codly can run locally (spec)

### 6.1 Stage A — retrieval-only commit replay ("CLTB-A"), no LLM, minutes

Implemented as a pilot in `scratchpad/scripts/commit_bench.py`; productize as `codly bench localize --repo . --n 300`.

1. **Select commits:** last N (default 300) **first-parent** commits on the default branch. Exclude: merges, reverts, bot authors (`[bot]`, dependabot, renovate), messages < 4 words after cleaning, commits whose retrievable gold is empty or > 25 files. Record every exclusion with its reason.
2. **Snapshot = exact parent:** index `parent(oldest)` once; after evaluating commit *cᵢ*, apply *cᵢ*'s diff to the index (delete/reinsert changed files). For first-parent linear history this reproduces each parent exactly and exercises the incremental indexer. (Never evaluate against HEAD — it leaks the answer.)
3. **Queries:** Q1 = subject line (default; closest to user prompts); Q2 = subject + body with trailers, PR numbers and URLs stripped (upper bound); Q3 (optional) = issue-style paraphrase written by a cheap model instructed not to name files/symbols. Report **leakage**: share of queries containing a gold basename or a gold symbol name.
4. **Gold:** files with status M/D/R at the parent, minus lockfiles, snapshots, generated, vendored, binaries. Line-level gold = changed hunks (`git diff -U0 parent c`) mapped to parent line numbers.
5. **Arms:** B0 naive grep (files containing the most query words), B1 paths, B2 symbols, B3 chunks, B4 RRF(B2,B3), B5 +graph rank, B6 +embeddings, B7 +rerank, plus any external tool via a common adapter (`jevgrep --json`, an MCP server) if the user opts in.
6. **Metrics:** file Recall@{1,5,10,20}, Hit@{1,5,10}, MRR, nDCG@10; **Span-Recall@budget** (fraction of gold hunks covered by returned spans within B tokens, B ∈ {1k, 4k, 8k}); pack tokens; latency p50/p95; **calibration**: Hit@5 per confidence level, coverage-vs-precision curve, ECE if a probability is emitted.
7. **Statistics:** per-commit paired differences; 95 % CI by 1,000× bootstrap over commits; Wilcoxon signed-rank for R@10 between arms. Report per-repo, never pool silently.
8. **Outputs:** JSON (all rows) + markdown summary; store under the repo's Codly data dir; use the confidence tables to **set that repo's thresholds** (§4.6).

### 6.2 Stage B — paired agent runs (token savings), costs quota

1. **Sample** M = 24 commits from Stage A, stratified by confidence level (8 high / 8 medium / 8 low) and gold size; prefer commits that touch tests (their tests become the hidden check).
2. **Task:** Q3 text in a fresh git worktree at the parent. Two task types: (i) *localize-only*: "list the files and line ranges that must change; do not edit" (cheap, scorable vs gold); (ii) *implement*: make the change; hidden check = the commit's own tests (FAIL→PASS) where available.
3. **Arms:** A0 plain CLI (empty MCP config via `--strict-mcp-config` / no `mcp_servers`); A1 MCP tools only; A2 pointer pack only; A3 pack + tools. Same CLI version, model, effort, machine; **R = 3 repeats per arm per task**, arm order randomized.
4. **Measure** from the CLIs' own streams: Claude Code `result` message (`num_turns`, `total_cost_usd`, `usage` with input / cache-creation / cache-read / output tokens); Codex `turn.completed.usage` (input, cached_input, cache_write_input, output, reasoning_output). Also tool calls by type, files read (count, tokens), wall-clock, localization F1, tests passed.
5. **Primary endpoint:** total billed-equivalent tokens per task (report components separately; for a single number weight cache reads at the provider's list-price ratio). **Claim savings only if** the paired 95 % CI excludes 0 **and** success is non-inferior (margin −5 pp). Show the per-task table — outliers like jevgrep's pytest task must be visible.
6. **Hygiene:** freeze Codly build + CLI versions; keep 1–2 holdout repos never used for tuning; publish exclusions; log retrieval cost (Codly CPU ms) alongside.

### 6.3 Pilot result to date

See §3.2: on `openai/codex`, 276 commits, subject-only queries → RRF Hit@5 0.65 / R@10 0.36 / MRR 0.50 vs naive grep 0.38 / 0.20 / 0.24; ranker agreement ≥ 2 predicts Hit@5 ≈ 0.85. Stage B not yet run.

---

## 7. Recommendations for Codly

1. **Core retrieval = local lexical + structural, no embeddings in v1.** `ignore` + tree-sitter tags (Rust, TS/TSX, JS, Python, Go, Swift first) + SQLite FTS5 (paths/symbols/chunks) + RRF + agreement confidence. Justification: §2.2 evidence + §3 measurements + zero runtime/model packaging.
2. **SQLite FTS5, not tantivy, to start** — one file, transactional with Codly's state, measured fast enough with AND-first shaping. Revisit tantivy only for chunk search if real repos exceed 150 ms p95.
3. **Static grammars only** (per-language crates). Don't ship `tree-sitter-language-pack` defaults (runtime download + dlopen conflicts with a notarized macOS app).
4. **Phase 1.5:** Aider-style personalized PageRank over def/ref as a 4th ranker and as a session "orientation map".
5. **Phase 2 (benchmark-gated):** transformer code embeddings (Jina v2 code via `fastembed`, or CodeRankEmbed/Qwen3-embedding via `ort`) in `sqlite-vec`/`usearch`. **Skip static potion-code** as a standalone ranker (CoIR 39 < BM25 42).
6. **Jev and other decision models: optional adapter only.** `DecisionProvider` trait; OpenRouter `/api/alpha/decisions` (reuses the user's OpenRouter key) or local Kev; disabled by default; explicit per-repo consent that source text leaves the machine; used only to rerank ≤ 20 candidates or to gate `medium` results. Measure it in Stage A/B like any other arm.
7. **Delivery = pointer pack + `codly-mcp` stdio sidecar**, wired per CLI as in §5.3; ≤ 4 tools; never use `--strict-mcp-config` outside benchmarks; don't rewrite agents' `Read` calls in v1 (use `additionalContext` hints).
8. **Confidence-aware by design:** at `low`, inject nothing but grep suggestions. Re-fit thresholds per repo from Stage A automatically after first index.
9. **Context isolation:** one MCP instance, budget ledger and worktree overlay per agent; pass citations (pointers), not contents, between agents.
10. **Explainability & world:** persist every retrieval with mechanical evidence (`retrieval_log`); `codly_explain_choice` powers the "why these files" panel; retrieval events drive the Living Agent World — real events only.
11. **Ship the benchmark** (`codly bench localize`) in v1; run Stage B before making any public token-savings claim.

---

## 8. Stale / unverified / open items

- Boris Cherny's "agentic search outperformed RAG" — only secondary reports; basis described as "mostly vibes" [Unverified].
- Cursor, Augment, Copilot, Cognition numbers are vendor-internal [Claim].
- Agentless/LocAgent/CoSIL/SweRank cross-paper numbers conflict; I quoted abstracts only.
- OpenCode `mcp` JSON shape, Cursor CLI / Copilot CLI / Pi MCP wiring — not verified today.
- Jev 1.14 latency, Kev/Laya/von quality — project-reported only; no local run.
- All Rust snippets were checked against crate sources but **not compiled** (no third-party build scripts were run, per the research rules). Measurements in §3 used system `rg` 15.1.0 and Python's stdlib SQLite 3.51.0, not the proposed Rust stack.
- Pilot benchmark: single repo; regex symbol extraction; file-level metrics only.

## 9. Sources

Primary (fetched/read 2026-10-25):
- TypeSafe docs: https://docs.typesafe.ai/models , https://docs.typesafe.ai/primitives ; org https://github.com/typesafe-ai
- OpenRouter Jev guide: https://openrouter.ai/docs/guides/community/jev.md ; models API https://openrouter.ai/api/v1/models
- https://github.com/BorisLeMeec/jev (`internal/typesafe/client.go`, `internal/run/{find,hook}.go`, `bench/RESULTS.md`, `bench/tokenecon/TOKENS.md`)
- https://github.com/nassim-arifette/jevgrep (`src/mcp.ts`, `src/evaluation/openrouter.ts`, `benchmarks/`)
- https://github.com/dzhng/jevgrep (`docs/architecture.md`, `evals/results/*.md`)
- https://github.com/AbdelStark/awesome-typesafe-jev ; https://github.com/jaredpalmer/kev ; https://github.com/shiftynick/jev-axi ; https://github.com/shimo4228/jev-skill-router ; https://github.com/rmax-ai/jevgrep-eval ; https://github.com/mikolajmikolajczyk/repoctx
- Claude Code docs: https://code.claude.com/docs/en/cli-reference , /hooks , /sub-agents , /mcp
- Codex source: https://github.com/openai/codex @ `970111f6` (2026-10-23): `codex-rs/core/*_prompt.md`, `codex-rs/file-search`, `codex-rs/config/src/mcp_types.rs`, `codex-rs/exec/src/exec_events.rs`
- Gemini CLI MCP docs: https://geminicli.com/docs/tools/mcp-server/ ; ACP: https://agentclientprotocol.com/protocol/session-setup
- Aider: https://github.com/Aider-AI/aider `aider/repomap.py`
- Anthropic, "Effective context engineering for AI agents" (2025-09-29): https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- Cursor: https://cursor.com/blog/semsearch (2025-11-06), https://cursor.com/blog/fast-regex-search (2026-03-23)
- Cognition: https://cognition.com/blog/swe-grep (2025-10-16)
- Augment: https://www.augmentcode.com/blog/context-engine-mcp-now-live (2026-02-06)
- GitHub: https://github.blog/news-insights/product-news/copilot-new-embedding-model-vs-code/
- ContextBench: https://arxiv.org/abs/2602.05892 ; SWE-bench: https://arxiv.org/abs/2310.06770 ; LocAgent https://arxiv.org/abs/2503.09089 ; SweRank https://arxiv.org/abs/2505.07849 ; SweRank+ https://arxiv.org/abs/2512.20482
- turbopuffer talk: https://www.ai.engineer/talks/zKk7sDMGDEQ-benchmarking-semantic-code-retrieval-on-claude
- potion-code-16M-v2 card: https://huggingface.co/minishlab/potion-code-16M-v2
- crates.io API for every version listed in §2.3/§4.2; crate sources for `libsqlite3-sys` 0.39.1, `tree-sitter-tags` 0.27.1, grammar crates, `tantivy` 0.27.0, `rmcp` 3.7.0, `fastembed` 7.5.0, `model2vec-rs` 0.3.0, `ignore` 0.4.33, `notify-debouncer-full` 0.7.0, `tree-sitter-language-pack` 1.22.1

Secondary (used only where flagged): eesel.ai / letsdatascience / datacamp on Jev; smartscope.blog, officechai on Claude Code RAG history; jacar.es on Cody embeddings removal; morphllm on Gemini subagents.

Local artifacts: scripts `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/{fts5_scale.py,commit_bench.py}`; results `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/out/{codex_bench.json,codex_bench_subject.json}` (scratchpad is session-scoped; copy into the repo if they should persist).
