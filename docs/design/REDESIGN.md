# Codly redesign brief (v0.2)

> **Update (v0.3):** the user rejected the codebase city ("I'd rather have the office"). The
> world is now a **cozy 3D low-poly office** (`src/office/`): characters walk between rooms
> that mean something — library, meeting room + whiteboard, glass Leads' office, desk pods,
> server room, mail room, memory archive, task board — driven only by real events. §6 below
> describes the retired city. Requests now run in parallel (one Lead per request thread).

User feedback on v0.1, verbatim intent:
1. The world copied the reference office too closely. Wanted: **something very different, in 3D, not an office**. Chosen direction: **the codebase city**.
2. UI/UX still needs work. All four problems confirmed: **too dense / cluttered**, **looks generic**, **hard to follow agents**, **chat & flow**.
3. Artifacts and image generation must be first-class.

The backend for this already exists: `api.repoMap()` (files with line counts and languages),
`Artifact` + `artifact_added` events + `Snapshot.artifacts`, `api.readArtifact(id)` (bytes;
MIME on the artifact). Codex image generation (`savedPath`) becomes an artifact automatically.

---

## 1. Product idea: an observatory over a living city

The repository is a city. Agents are small luminous drones flying between the real files
they read and write. The user watches from a calm observatory and talks to the Lead. Work
panels are glass sheets floating over the city, not a wall of tabs. One selection drives
everything: click an agent, a task or a file anywhere and every surface focuses on it
(the camera flies there, the sheet opens, the timeline scrolls).

This replaces "Workspace vs World" with **one home** where the city is the backdrop and the
conversation is the foreground. A focus toggle (⌘. ) dims/blurs the city for reading; ⌘1/⌘2
keep a full-city view and a full-work view for people who prefer them.

## 2. Information architecture (fixes "too dense")

From 9 tabs + 3 sidebars to **one home and four rooms**, reached from a slim icon rail:

| Rail | What | Replaces |
|---|---|---|
| **Home** | City backdrop + the Lead conversation (centered column, max ~720 px) + the crew strip | Chat, World, Split, Inbox |
| **Plan** | One canvas: the task DAG flowing left → right; each task card carries its agent, state and progress. Lenses: Flow (default) · Board · Timeline | Board, Graph, Timeline |
| **Changes** | Per-plan and per-task diffs + the **Artifacts** gallery | Diffs |
| **Knowledge** | Team memory (shared / private / task), knowledge graph, code search | Memory |
| **Terminal** | Shell in the repo or any worktree | Terminal |

Everything else is on demand: the **agent/task sheet** slides in from the right only when
something is selected; Settings is a sheet; ⌘K reaches everything.

Remove: the persistent second sidebar listing threads, the always-on inspector, the
duplicate status texts. Each fact appears in one place at a time.

## 3. Following agents (fixes "hard to follow")

- **Crew strip** (top of Home): one pill per active agent — portrait (the agent's drone),
  name, provider, and ONE plain-language sentence derived from events (no LLM):
  "Reading `auth.ts`", "Running `pnpm test` · 0:41", "Waiting for you: `rm -rf dist`",
  "Done — +48 −6 in 2 files". A thin ring shows its todo-list progress. Amber when it needs you.
- **Narration rules**: sentence = verb + target + since; tests show counts; approvals say
  exactly what is asked. Same sentence in the crew strip, the city label and the sheet.
- **Agent sheet**: Now (sentence + Why? with provenance), Needs you, Task (goal, criteria
  with live checks), Steps (its own todo list), Files (read / modified, with why), Output
  (live), Cost & context, History. Collapsed sections remembered.
- **Subagents** appear as small companions on their parent (city + crew strip), with their
  description; clicking shows the parent's sheet scrolled to them.
- **Cross-highlight**: hovering an agent highlights its task card, its files in the city and
  its lane; hovering a file highlights who touched it.

## 4. Conversation & flow (fixes "chat & flow")

The Lead conversation is the spine of the product; everything important lands in it as a
rich, actionable card, in order:

1. **Plan card**: summary, the mini DAG (tasks as chips with their assigned provider), open
   questions highlighted, primary action **Run plan ⌘↵**, secondary Edit (ask the Lead),
   Cancel. Once running it becomes a live progress header pinned at the top of the chat.
2. **Approval card** (inline, where it happened): exact command / diff, why, inferred risks,
   choices exactly as offered, keyboard ⌘↵ / ⌥⌘↵ / ⌘⌫. The Inbox is just a filter of these.
3. **Task done card**: who, what, diffstat, verify result, review verdict, artifact
   thumbnails, "View changes".
4. **Plan done card**: summary, total diff, artifacts, **Merge into `<branch>`** / Keep branch.
5. **Questions from agents** (codly_ask_lead) and the Lead's answers, attributed.

Composer: compact (one line, grows), `@agent` to talk 1:1 (switches the thread, clearly
shown as a chip), `#task` references, suggested next actions as small chips under the last
message ("Run plan", "View changes", "Merge"). Streaming text renders progressively without
layout jumps. 1:1 chats with agents use the same component, reached from the crew strip.

## 5. Visual identity (fixes "looks generic")

Not another gray Linear clone. Direction: **"Observatory"** — calm, nocturnal, editorial,
with light that means something.

- **Palette**: dark (default) is a deep blue-black, not gray (e.g. `#0A0D14` base, panels
  `rgba(18,22,32,0.72)` glass with a 1 px inner highlight); light is warm paper
  (`#F5F2EC`). One signature **aurora** accent (teal → violet gradient) used *only* for
  live activity and the primary action. Agent identity hues carry meaning everywhere
  (their drone, lane, task card edge, diff gutter, city buildings they touched).
- **Typography**: an editorial serif for display moments — plan titles, empty states, the
  welcome — `Instrument Serif` (OFL, `@fontsource/instrument-serif`) or `Fraunces`
  (`@fontsource-variable/fraunces`); Inter for UI; JetBrains Mono for code. Generous line
  height, fewer sizes.
- **Space**: more air, fewer borders; group by spacing and soft surfaces. Density only where
  scanning matters (diffs, tables).
- **Motion**: physical and meaningful — cards move between plan stages with springs, the
  camera eases to selections, nothing loops forever. Respect reduced motion.
- **Personality**: one line of copy in empty states, in the serif. No emoji confetti in chrome;
  delight lives in the city.
- Keep: keyboard-first, ⌘K, provenance badges (Observed / Inferred / Provided /
  Model-stated), honest cost labels.

## 6. The 3D codebase city

- **Engine**: three.js 0.186, framework-free module in `src/city/` + `CityView.svelte`.
  **Render on demand** (render only while something animates or the camera moves; zero
  frames when still), DPR capped at 2, context-loss recovery, instancing for buildings.
- **Layout**: squarified treemap of the directory tree → districts (plots with streets
  between them, ground tinted per top-level folder, labels as DOM overlay); files →
  buildings sized by lines (footprint from the treemap cell, height ∝ log lines). Language →
  material palette (cohesive, desaturated). Large repos: aggregate small files per directory
  above ~3,000 files. A central **integration spire** (the plan's integration branch) and a
  **memory crystal** (team memory) sit in a plaza; the Lead's **observatory** hovers above.
- **Agents**: small rounded drones with an identity-colored glow, expressive eyes, gentle
  bob; trails when flying. Subagents = mini drones orbiting their parent.
- **Real events → visuals** (honest: no fake activity; ≤ 250 ms to visible state):
  read → drone hovers at the building, scan beam; search → radar ring over the district;
  edit/create → building glows in the agent's color with construction sparks; new file →
  building rises; delete → sinks; command → drone at a terminal plaza with a shell glyph;
  tests → lightning tower, pass = fireworks, fail = red smoke; approval → tall amber beacon
  above the drone visible from anywhere; blocked → rain cloud; rate-limited → docked, zZ;
  note / message / ask-lead → light packet arcs to the observatory; memory write → packet
  into the crystal; merge → beam from the task's district to the spire; task write scope →
  glowing outline on the districts it owns (overlaps visible); artifacts → small billboard
  above the building that produced them.
- **Camera**: orbit / pan / zoom, smooth fly-to on selection, default 3/4 angle that frames
  the active districts; keyboard (arrows, +/−, F follow, Esc).
- **Light & theme**: dark = night city (lit windows, glowing drones); light = soft day.
- **Accessible**: off-screen agent list synced with selection; ticker mirrored to aria-live.

## 7. Artifacts

Gallery in Changes and thumbnails in chat cards: images (incl. generated), SVG, HTML
(sandboxed iframe, `sandbox=""` with blob URLs — no scripts by default, opt-in toggle "run
scripts" with `allow-scripts` only), Markdown (rendered, sanitized), PDF (embedded), CSV
(table). Each shows who made it, in which task, with "Reveal in Finder" and "Copy path".
Bytes via `api.readArtifact(id)` → Blob URL (revoke on unmount).

## 8. Non-goals for this pass

Live app preview servers, voice, a layout editor for the city.
