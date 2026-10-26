# Munder Difflin: office visualization and UI

Research notes for the Tauri + Rust successor. Date: 2026-10-25.

**Source:** shallow clone of Munder Difflin, package version 0.4.6, MIT source license. Clone path: `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/research-src/munder-difflin`.

**Method:** static reading only. I did not run the app, its scripts, binaries, or npm. Numbers such as `OfficeFloor.tsx:247-261` are line references into the clone. Paths without a prefix are under `src/renderer/src/`. Other areas are covered by sibling reports in this folder (`coordination.md`, `runtime.md`, `features.md`), which I did not read.

**Not measured:** frame rate, CPU, GPU, and visual behaviour. Where I describe what a screen looks like, I say whether I read the code or saw a screenshot.

---

## Answers in brief

1. **Rendering.** PixiJS 8 (lockfile 8.18.1) inside Electron 32. Maps are Tiled JSON (`.tmj`) with 16 px tiles, parsed by a custom renderer, one Sprite per non-empty cell. The camera fits the whole map to the pane with a non-integer zoom. A ticker callback runs every frame while the floor is visible. **It does not idle.** It stops only when fullscreen, the IDE, or a hidden window pauses it.
2. **Characters.** Procedurally painted 18×32 px frames from per-character recipes. No sprite sheets are shipped. Walking is BFS over the tile grid with straight-line interpolation at 48 px/s. Seats go to whoever arrives next. The `type` and `read` animations are dead code.
3. **Events.** Real status comes from Claude Code hook events (`hooks/useHive.ts`). A PTY screen scraper (`hooks/usePtyParser.ts`) refines the action text, but only while that agent's terminal view is mounted. Bubble prefixes are tool glyphs: `<` Read, `>` Edit/Write, `$` Bash, `?` Grep/Glob, `@` web, `=` TodoWrite, `*` other. Two corrections to the brief: running tests shows `$`, and reading a file shows `<`. "heading to web" exists only in the mock. The mock runs whenever no agent has a live PTY. Cafeteria, coffee, errands, gossip, steam, and monitor animations are filler.
4. **Themes.** Six ids are declared, two are registered (`office` and a Brooklyn 99 placeholder), and four fall back to the office. Switching theme kills every non-god agent's PTY. There are no seasonal decorations in the app. Halloween assets belong to the marketing site. There is no day/night cycle.
5. **Licensing.** Source is MIT. The tilesets are LimeZu art with a credit requirement and an unclear redistribution term, and the credit does not appear in the app. Engine files are ported from `shahar061/the-office` under ISC, and the ISC notice is not in the repo. The cast uses The Office character names and quotes, and other shows supply theme names.
6. **UI.** Title bar, office floor on the left, agent sidebar on the right (or the boss's Command Center, with eleven tabs), roster strip at the bottom, and full-window overlays for focus terminal and IDE. Floor and panels sync through the zustand store, plus floor landmarks that open Command Center tabs. Tokens are CSS variables mirrored in TypeScript, with a 4 px grid, Press Start 2P, Inter, and JetBrains Mono, and light and dark themes.
7. **Critique.** Status is duplicated across four places, text is small and zoom-dependent, the map letterboxes with no pan or zoom, reduced motion is ignored, the canvas is invisible to assistive tech, some copy is hard-coded English, and `DESIGN.md` has drifted from the code and is styled as a game HUD. Priorities for the successor: licensing, one state machine, dirty-driven rendering, DOM text, and filler off by default. Details in section 7.

---

## Contents

1. Rendering stack, map, camera, render loop (Q1)
2. Characters (Q2)
3. From agent events to visuals (Q3)
4. Themes, seasonal decorations, day and night (Q4)
5. Assets and licensing (Q5)
6. UI layout, synchronization, design tokens (Q6)
7. Critique and recommendations (Q7)
8. Not verified
9. Files read

---

## 1. Rendering stack, map, camera, render loop (Q1)

### 1.1 Stack and initialisation

- `package.json` declares `pixi.js ^8.5.1`, `react ^18.3.1`, `zustand ^4.5.5`, `electron ^32.2.0`, `@xterm/addon-webgl ^0.19.0`. `package-lock.json` pins `pixi.js` 8.18.1 and `electron` 32.3.3.
- Pixi needs an eval workaround for Electron's CSP (`scene/office/OfficeFloor.tsx:4-5`):
  ```ts
  // PixiJS uses new Function() internally, blocked by Electron CSP — this patches it.
  import 'pixi.js/unsafe-eval';
  ```
- The application is created and initialised in one effect (`OfficeFloor.tsx:234`, `:247-261`):
  ```ts
  await app.init({
    background: hexNum(theme.palette.background),
    antialias: false,
    roundPixels: true,
    resolution: Math.max(window.devicePixelRatio || 1, 2),
    autoDensity: true,
    ...
  });
  ```
  The resolution floor of 2× is intentional. The comment (`:251-256`) says it keeps bubble text legible at 100% scaling. The cost is four times the fill work on a 1× display.
- Textures load through an `<img>` element and are set to nearest-neighbour sampling (`OfficeFloor.tsx:134-145`). The comment explains that Pixi's `Assets` cannot type-detect the inlined data-URL tilesets.
- The whole Pixi application is torn down and rebuilt whenever `officeTheme`, `glGeneration`, or `i18n.language` changes (`OfficeFloor.tsx:1774`). Switching the UI language therefore reloads the map and every texture.

### 1.2 Map format, tiles and scale

- **Format:** Tiled JSON (`.tmj`), orthogonal, right-down render order. `assets/maps/office.tmj` is 34×22 tiles of 16×16 px, saved by Tiled 1.12.0. `assets/maps/brooklyn99.tmj` is 36×24 tiles, saved by Tiled 1.10.2. Both are imported as raw text (`scene/office/themeRegistry.ts:29-30`) and parsed by `TiledMapRenderer.ts`. The project does not use `pixi-tiled`.
- **Layers:** tile layers `floor`, `walls`, `furniture-below`, `furniture-above`, plus a `collision` tile layer. Object groups `spawn-points` and `zones`. Non-empty cells in `office.tmj`: floor 576, walls 266, furniture-below 156, furniture-above 148, collision 447.
- **Rendering:** each non-empty cell becomes one `Sprite` (`TiledMapRenderer.ts:196-249`). There is no baked layer, no `ParticleContainer`, no `cacheAsTexture`, and no `BitmapText` anywhere under `scene/office`. That is fine for about 1,150 sprites, and it does not scale.
- **Spawn points** are named seats and stations: `desk-ceo`, `pc-1` to `pc-6`, `desk-chief-architect`, `desk-team-lead`, `desk-backend-engineer`, `desk-product-manager`, `desk-data-engineer`, `desk-project-manager`, `desk-market-researcher`, `desk-agent-organizer`, `desk-ui-ux-expert`, `warroom-seat`, `entrance`, `cafe-seat-1` to `cafe-seat-4`, `cafe-stand-coffee`, `cafe-stand-vending`. **Zones:** `boardroom` (9×5 tiles) and `cafeteria` (8×8 tiles).
- **Tile size in code:** `TiledMapRenderer.ts:71` uses `mapData.tilewidth`, which is 16. `design/tokens.ts:65` exports `tileSize = 32` with the comment "the world is built from 32×32 tiles". Nothing imports it, and `DESIGN.md:173` repeats the 32 px grid. Both are wrong for the shipped maps.

### 1.3 Camera

- `Camera.ts:45-56`: `fitToScreen()` sets the zoom to `min(viewW / mapW, viewH / mapH)`. The whole map is always visible and the zoom is a float.
- `Camera.ts:8` sets `const LERP_SPEED = 0.08;`. `update()` applies it once per frame (`Camera.ts:76-78`), not scaled by `dt`. Convergence is therefore twice as fast on a 120 Hz display as on 60 Hz.
- Clamping and letterboxing: `Camera.ts:97-108`. Transforms: `Camera.ts:93-95`.
- The only camera motion is a nudge toward the selected agent. `OfficeFloor.tsx:1594-1603` subscribes to `selectedId` and calls `camera.nudgeToward`. There is no wheel, drag, or keyboard zoom. `Camera.focusOn` (`Camera.ts:59-64`) has no callers.
- **Estimate, not measured:** on a laptop-sized floor pane the fit zoom is roughly 1.5 to 2.5. It is not an integer, which `DESIGN.md:179` forbids ("never 1.5×"), and nearest-neighbour sampling at a non-integer scale produces uneven pixel columns.

### 1.4 Render loop, and whether it idles

- One ticker callback (`OfficeFloor.tsx:1683-1706`), registered at `OfficeFloor.tsx:1707`:
  ```ts
  const onTick = (ticker: Ticker) => {
    const dt = ticker.deltaMS / 1000;
    camera.update(dt);
    ...
    updateCafeteria(dt); updateCoffeeRuns(dt); updateErrands(dt);
    updateBossAura(dt); updateDeskLife(dt); updateBoardMoves(dt);
    resolveBubbleOverlaps();
    ...
  };
  app.ticker.add(onTick);
  ```
- There is no dirty flag. Nothing checks whether anything moved. Pixi renders on each ticker tick unless configured otherwise, and this init does not configure it.
- **Pause:** `OfficeFloor.tsx:209-226`:
  ```ts
  const paused = !!fullscreenAgentId || ideOpen || docHidden;
  ...
  if (paused) ticker.stop(); else ticker.start();
  ```
  The comment at `OfficeFloor.tsx:185-208` explains that the scene and its textures stay alive while paused, so resuming is instant.
- **Answer:** the floor does not idle. With every agent seated and quiet, it still renders every frame while visible. It stops only when the document is hidden, a fullscreen terminal covers it, or the IDE is open.
- Movement uses real time. `Character.ts:895` uses `SPEED * dt`. The comment at `OfficeFloor.tsx:206-208` says Pixi clamps the delta after a long pause, so a floor paused for an hour does not teleport avatars.

### 1.5 Performance tricks and problems

Tricks that are there:

- Pause when hidden, fullscreen, or in the IDE (1.4).
- Frame-independent walking (`Character.ts:895`).
- Texture caches: `cast.ts:55` (`frameCache`), `portraitArt.ts` (`bufCache`, `sceneCache`).
- Change detection in `applyState` (`OfficeFloor.tsx:1449-1455`), so store writes that change nothing do no work.
- Pure policy module for GL recovery (`glRecovery.ts`), covered by `test/office-gl-recovery.test.cjs`. It is the only office test in the repo.

Problems (facts from the code; severity is my judgement):

1. **Always-on rendering** while visible (1.4). High on laptops.
2. **Per-frame `Graphics` rebuilds** where a static or pre-baked texture would do:
   - `DeskScreen.update` clears and redraws every frame while seated (`DeskScreen.ts:64-80`).
   - Parked-cup steam is redrawn every frame (`Character.ts:711-720`).
   - Carried cup and steam while carrying (`Character.ts:754-760`).
   - Status overlay cleared and redrawn every frame while a glyph is active (`Character.ts:820-825`).
   - Ask-me board redrawn every frame while questions are pending (`OfficeFloor.tsx:1287`, `:1155-1178`).
   - Errand ambient effects every frame (`OfficeFloor.tsx:881`).
   - Sink and coffee-machine steam while busy (`OfficeFloor.tsx:474-507`).
3. **Text objects.** Each avatar has a nameplate `Text` (`Character.ts:195`) and a thought-bubble `Text` (`ThoughtBubble.ts:81`). The thinking dots change the label about twice a second (`ThoughtBubble.ts:213-221`). Each text change re-rasterises a canvas texture.
4. **Store churn.** `updateAgent` copies the whole `agents` array on every call, with no equality check (`store/store.ts:699-712`). The PTY parser calls it on each terminal chunk (`usePtyParser.ts:126-131`, `:144`). The floor re-syncs every avatar whenever the `agents` reference changes (`OfficeFloor.tsx:1594-1596`, `syncAgents` at `:1575-1589`).
5. **Polling.** Three independent five-second pollers read the same ledger, plus one more fetch:
   - `OfficeFloor.tsx:1377` (`pollTaskBoard`, every 5 s, whole ledger via `window.cth.hiveTasks()`).
   - `components/AgentStrip.tsx:80` (doing-notes on the roster cards, every 5 s).
   - `components/MemoryGraphPanel.tsx:44` (graph refresh, every 5 s).
   - `OfficeFloor.tsx:951-953` (boss-aura counts, every 30 s).
6. **Per-tile sprites** for about 1,150 static cells (`TiledMapRenderer.ts:196-249`), with no batching or caching.
7. **Non-integer zoom** with nearest sampling (1.3).
8. **`Array.shift` in BFS** (`pathfinding.ts:33`). Negligible on a 34×22 grid, but avoidable.
9. **Full rebuild on language change** (`OfficeFloor.tsx:1774`).
10. **Terminal WebGL pressure.** Each attached xterm takes a WebGL lease (`components/terminalPool.ts:602` creates `WebglAddon`, `:775` leases it on attach, `:794` releases it on detach). Chromium caps live WebGL contexts (about 16) and evicts the oldest, which is the floor's. The recovery is in place (1.6), and the release notes describe the incident (`CHANGELOG.md:543`). The root cause is the number of attached terminals.

### 1.6 WebGL context loss

- The problem and the fix are described in `glRecovery.ts:1-20`. Pixi reports nothing when its context is evicted, so the floor goes blank.
- `installContextLossRecovery` calls `preventDefault()` on `webglcontextlost` (`glRecovery.ts:56-60`) and then requests a rebuild through a React generation bump (`glRecovery.ts:68-70`, `OfficeFloor.tsx:266-277`). The retry budget is capped at three rebuilds (`glRecovery.ts:39`).
- Init retries cover a GPU process that is still restarting: three attempts at 1500 ms each (`glRecovery.ts:40`, `:112`, `:146-157`).
- **Judgement:** good recovery, and the pure-function structure is testable. The better fix is to limit how many xterm WebGL leases can exist at once, so the floor is never the one evicted.

---

## 2. Characters (Q2)

### 2.1 How the sprites are made

- No sprite sheets ship for the cast. Frames are painted procedurally from per-character recipes into pixel buffers (`portraitArt.ts:1-10` describes the recipe layering: skin, clothing, face, facial hair, hairstyle, glasses). Scene buffers come from `sceneFrameBufs` (`portraitArt.ts:565-575`), and `cast.ts:57-67` converts them to textures.
- Frame size: `SCENE_W = 18`, `SCENE_H = 32` (`portraitArt.ts:15-16`). Portrait size: 18×28 (`:12-13`).
- Each character has three walk frames (stand, step-left, step-right) for front and back views (`cast.ts:69-88`). Rows are `down` (front), `up` (back), and `right`, which reuses the front view (`cast.ts:85`). `left` is the right row with `scale.x = -1` (`CharacterSprite.ts:98`). **Sideways walkers therefore face the viewer, mirrored.**
- The scene scale is `CHAR_SCALE = 1.08` (`CharacterSprite.ts:24`, `:53`). An avatar is 32 px × 1.08 = 34.56 px tall, which is about two 16 px tiles.
- The old comments say the walk frames are LimeZu recolours. Those comments are stale. See 5.6.

### 2.2 Animation frames and states

- Frame index sequences (`CharacterSprite.ts:14-19`):
  ```ts
  const ANIM_FRAMES: Record<AnimState, number[]> = {
    walk: [0, 1, 2, 1],
    type: [0, 1, 2, 1],
    read: [0, 1, 2, 1],
    idle: [0],
  };
  ```
  Playback speed (`CharacterSprite.ts:99`): walk 0.15, idle 0.08, others 0.06.
- **`type` and `read` are dead.** `Character.ts:14` declares them, and `pendingWork` (`Character.ts:107`) is only ever assigned `null` (`:278`, `:293`, `:341`, `:370`, `:385`). Nothing selects a typing or reading pose. The floor has no typing animation at all.
- **Pose and effect states on `Character`:**
  - idle and walk (`Character.ts:855-901` for walking, `:905-958` for the idle loop and wander);
  - seated at a desk, with a leg-crop mask and a body offset (`Character.ts:42-50`, `:285-310`, and the mask in `CharacterSprite.ts:62-84`);
  - status overlays drawn above the head (`Character.ts:820-853`): `blocked` a blinking `!` (`:826-831`), `success` a four-point sparkle for 0.9 s (`:832-838`), `compacting` a violet box that pulses (`:839-843`), `looping` an orange ring with a spinning lit dot (`:844-851`). The glyph type is `StatusGlyph` (`Character.ts:15`) and has no `attention` value, even though `DESIGN.md:450` lists one;
  - cheer: 1.6 s of hops and 14 confetti pieces (`Character.ts:461-481`, `:733-751`);
  - cigar and watering, both timed (`Character.ts:547-561` and `:764-789` for cigar, `:526-541` and `:793-817` for watering);
  - carried cup with steam (`Character.ts:754-760`);
  - ghost: alpha 0.5 (`OfficeFloor.tsx:1474`).

### 2.3 Pathfinding and movement

- BFS over the four-neighbour collision grid (`pathfinding.ts:15-53`). The file header says it is "Ported verbatim from shahar061/the-office".
- Movement is straight-line interpolation through the path's tile centres at `SPEED = 48` px/s (`Character.ts:33`, `:880-900`). The target is the tile centre horizontally and the tile bottom vertically, so feet sit at the tile edge (`Character.ts:882-883`).
- **No avatar-to-avatar avoidance.** The path only consults the collision grid (`Character.ts:236-245`), so two avatars can overlap as they pass. The comment at `Character.ts:70-76` concedes that vertical neighbours always overlap.
- Spawns are forced walkable by name prefix (`TiledMapRenderer.ts:66`, `:157-164`): `desk-`, `pc-`, `warroom-`, `entrance`.
- Wandering picks a random walkable tile within six tiles, trying up to 14 candidates (`Character.ts:940-958`). The pause between legs is 1 to 4 s (`Character.ts:867`, `:944`).

### 2.4 Desk assignment

- The seat list has 16 named primary seats (`themeRegistry.ts:153-159`), followed by the walkable tiles of the `boardroom` zone as overflow (`OfficeFloor.tsx:335-354`).
- The claim policy is first free index, and index 0 is reserved for the god agent (`OfficeFloor.tsx:380-387`). Seats are released on removal (`OfficeFloor.tsx:1440`).
- **Seat names are software-role labels** (`desk-chief-architect`, `desk-backend-engineer`, and so on), but nothing maps an agent's role to its seat. Agents take whichever seat is free. A cast member named Dwight can sit at `desk-chief-architect`.
- Facing: `facingForSeat` looks at the four neighbours of a seat and turns the agent toward its furniture (`OfficeFloor.tsx:394-400`). With a desk to the north, the agent faces up, so the viewer sees its back.
- **Desk monitor.** `DeskScreen` overlays the lit 2×2 monitor block while its owner is seated (`DeskScreen.ts:14-25`, created at `OfficeFloor.tsx:1413-1419`, toggled at `:1009-1012`). The screen animation (scrolling lines and a blinking cursor) is decorative and not tied to terminal output.

### 2.5 Z-order

Inside `rootContainer` (`TiledMapRenderer.ts:188-256`):

1. `floor`, `walls`, `furniture-below`, `furniture-above`, drawn in order with no sorting.
2. `characterContainer`, added last, with `sortableChildren = true` (`TiledMapRenderer.ts:74`, `:255`).

Inside the character container:

- avatar body: `zIndex = py` (`Character.ts:643`);
- nameplates: `NAME_TAG_Z + py`, with `NAME_TAG_Z = 50000` (`Character.ts:77`, `:649-650`);
- thought bubbles: `100000` (`ThoughtBubble.ts:70`). The `ToolBubble` class also uses that value (`ToolBubble.ts:65`), but the class is unused (6.7);
- desk monitor overlay: `(top.y + 2) * tile - 1` (`DeskScreen.ts:51`);
- message envelopes: `1_000_000` (`MessageEnvelope.ts:71`).

**Consequence:** every avatar and effect is drawn above every map layer, including `furniture-above`. Only the desk-monitor overlay is depth-sorted against avatars. An avatar walking behind a tall piece of furniture-above art will draw over it. This follows from the layer order. I did not check it visually.

### 2.6 Idle behaviour (what idle agents do)

- **Idle loop:** 30 s lingering on the floor, then 30 s resting at the desk, timer-driven (`Character.ts:56-57`, `:905-938`). It starts once the agent is idle and measures nothing about the work.
- **Cafeteria:** an idle or succeeded agent is sent to a café spot about every 6 to 12 s, with a 70% chance per window and at most four agents at once (`OfficeFloor.tsx:735-794`). A break lasts 8 to 16 s (`OfficeFloor.tsx:722`). Solo quips and two-beat exchanges come from `cafeteriaLines.ts`, which is plain English strings, not i18n keys.
- **Coffee economy:** four clean mugs in stock (`themeRegistry.ts:171`, `OfficeFloor.tsx:439-600`). Brewing takes 2.6 s and washing 2.4 s. An agent carrying its used mug refills it 60% of the time and washes it 40% (`OfficeFloor.tsx:681-683`). The stock is internal. It does not reflect any work.
- **Errands:** a 14 to 32 s cooldown and a 65% chance, for idle agents only (`OfficeFloor.tsx:868-937`). Spots are in `themeRegistry.ts:178-200`: plant watering, two windows, dispensers, fridge, shelf, bins, and the boss's cigar at his window (`godOnly`).
- **Boss aura:** when an idle worker is within 44 px of the god avatar, it says a line (`OfficeFloor.tsx:939-992`, `:982`). Each worker gets one line per 25 s, with a 60% chance (`:983-984`). The done-task count comes from the ledger (`:951-953`, `:959-963`). The copy is in `i18n/locales/en.json:749-757`: "already shipped {{done}} tasks, {{godName}}. raise?" and similar lines.
- **Gossip** plays when the agent is out of earshot, more than 96 px from the boss (`OfficeFloor.tsx:611-624`). Examples in `en.json:758` onwards: "has he ever actually written code?" and "another 'quick sync' that took an hour…".
- **Cheer:** only after a busy stretch of at least 60 s (`OfficeFloor.tsx:83`, `:1464-1466`), so short inbox nudges end quietly (comment at `:79-82`).

**Real work always wins.** When an agent's status stops being idle or success, `applyState` cancels any break, errand, or coffee run (`OfficeFloor.tsx:1480-1504`). That rule is right and worth keeping.

---

## 3. From agent events to visuals (Q3)

### 3.1 Sources

| Source | Where | What it writes | What the floor does with it |
|---|---|---|---|
| Claude Code hooks: PreToolUse, PostToolUse, UserPromptSubmit, Stop, SubagentStop, Notification, PreCompact, PostCompact; Antigravity: PreInvocation, PostInvocation | registered in `main/hive.ts:1295-1305`; consumed in `hooks/useHive.ts:479-554` | `status`, `action`, `carrying`, `currentStation` | authoritative status |
| PTY screen scrape | `hooks/usePtyParser.ts` | `status`, `action`, `currentStation`, `carrying`, `blockReason` | refines action; sets blocked or waiting from prompt text |
| Circuit breaker | `useHive.ts:556-563` | `status: 'looping'`, `action: reason` | orange ring glyph |
| Hive ledger (`tasks.json`) | `OfficeFloor.tsx:1312-1375`, every 5 s | nothing in the store | task-board choreography |
| Hive message routing | `main/hive.ts:1790-1801` emits `hive:message`; `preload` relays; `OfficeFloor.tsx:1632-1636` consumes | nothing in the store | message envelopes |
| Mock loop | `store/mockEvents.ts`, started from `App.tsx:210-213` | `status`, `action`, `carrying`, `currentStation` | fake activity |

The code states its own priority. `useHive.ts:488-489`: "Hook events are the authoritative status source for real agents (the pty-stream parser only refines the on-floor action/station)."

Two gates decide which sources reach a given floor:

- **Mock gate** (`App.tsx:210-213`):
  ```ts
  const DEMO = import.meta.env.DEV && import.meta.env.VITE_CTH_DEMO === '1';
  ...
  if (DEMO || !hasLive) startMockLoop();
  ```
  The mock runs whenever no agent has a live PTY, and in the dev demo mode. The comment (`App.tsx:203-206`) says Michael always spawns, so the mock "effectively never runs" in normal use. But restored agents without a PTY are animated by it, because the mock steps every agent that lacks a `ptyId` (`mockEvents.ts:155`).
- **Parser gate:** the scraper is attached only while a terminal view is mounted for that agent. That is the sidebar terminal tab (`AgentDetailPanel.tsx:93`, `:253`), the Command Center terminal tab (`CommandCenterPanel.tsx:129`, `:303`), or focus mode (`FullscreenTerminal.tsx:170`, `:593`). `PtyTerminalView.tsx:150` attaches the callback and `:268` clears it on unmount. A worker that is not selected, or whose terminal tab is closed, is driven by hook events alone.

### 3.2 Status to floor behaviour

From `applyState` (`OfficeFloor.tsx:1449-1573`; the switch starts at `:1509`):

| Store status | Floor behaviour | Refs |
|---|---|---|
| `working`, `thinking` | sits at desk with focus glow; thought is the live action with its tool glyph; empty action shows animated dots | `:1510-1515` |
| `waiting` | sits at desk, no glow; thought is the action, or "waiting" | `:1516-1523` |
| `blocked` | blinking `!`; thought is the action, or "needs you"; walks to a waiting tile near the entrance | `:1524-1528`, waiting tiles `:359-376` |
| `compacting` | violet box glyph; sits; thought "compacting context" | `:1529-1535` |
| `looping` | orange ring glyph; sits without glow | `:1536-1542` |
| `success` | sparkle; wanders; cheers if the busy stretch was 60 s or more | `:1543-1553` |
| `ghost` | half opacity; idle; hides thought | `:1554-1558`, `:1474` |
| `idle`, and anything else | the god sits at its desk; workers wander, thought is the action or "idle"; cheer if finished work | `:1559-1571` |

`typing` is not a store status. `AgentCard.tsx` and `FullscreenTerminal.tsx:770` derive it from unsent text on the prompt, and it never reaches the floor. An agent typing a draft therefore looks idle on the floor.

### 3.3 Hook events to store

From `hooks/useHive.ts:488-554`:

- `PreCompact` sets `compacting` with action "compacting context" (`:490-492`).
- `PostCompact` sets `working` with action "resumed" (`:493-494`).
- `PreToolUse` sets `working`, with `currentStation` and `carrying` from `stationForTool`, and action `using <Tool>` (`:495-498`).
- `PostToolUse` and `UserPromptSubmit` set `working` without changing the action (`:499-502`).
- `PreInvocation` (Antigravity) sets `working` with action "thinking" (`:503-505`).
- `PostInvocation` (Antigravity) sets `idle` (`:506-511`).
- `Stop` and `SubagentStop` set `idle` with action "idle". If the stop was blocked to re-engage the inbox, they set `working` with action "reading inbox" instead (`:512-521`).
- `Notification` (`:522-545`): text about permission, approval, confirmation, or "needs your" sets `blocked` for the god and `waiting` for workers. Idle notifications set `idle`.

Tool to station and carry (`useHive.ts:197-228`): Read to shelf, Edit and Write to desk, Bash to terminal, Grep and Glob to shelf, WebFetch and WebSearch to web, TodoWrite to board, Task to mailbox (carry TodoWrite), and `mcp__*` to mcp. Unknown names fall back to a regex, then to desk. The carry value only changes the bubble glyph, because the floor never reads the station.

### 3.4 PTY screen scrape

`hooks/usePtyParser.ts`:

- Tool-line regex (`:6`): `const TOOL_RE = /●\s+([A-Za-z][A-Za-z_]*)(?:\s+(.+))?/g;`. It matches the Claude Code TUI's tool lines.
- Summary text (`:122`): `${lastTool.toLowerCase()} ${lastArg}`. The bubble therefore reads "bash npm test", not "running tests".
- While "esc to interrupt" is on screen, the agent stays `working` (`:106`, `:142-146`).
- **Idle:** four seconds without a tool line sets `idle` with action `awaiting` (`:57-69`, `:181`). That is the source of the word "awaiting".
- **Blocked:** prompt text in the last 400 characters matching `BLOCK_HINTS` (`:29-35`, `:149-150`). For the god, this sets `blocked` with approve and deny actions (`:154-168`). For workers, it sets `waiting` with action "waiting on god" (`:169-176`).

The file calls itself "a stopgap until we wire real Claude Code hooks" (`:44-46`).

### 3.5 Prefix glyphs

The glyph comes from `carrying`, not from the status:

- `ThoughtBubble.show(text, tool)` renders `${toolIcon(tool)} ${text}` (`ThoughtBubble.ts:103-117`, specifically `:110`).
- The map is in `ToolBubble.ts:7-18`:

| Tool | Glyph |
|---|---|
| Read | `<` |
| Edit, Write | `>` |
| Bash | `$` |
| Grep, Glob | `?` |
| WebFetch, WebSearch | `@` |
| TodoWrite | `=` |
| MCP, and anything else | `*` |

**Corrections to the brief's examples:**

- "@ running tests" would display as "$ running tests", because the Bash tool maps to `$` (`mockEvents.ts:33`). `@` is web.
- "> reading SPEC.md" would display as "< reading SPEC.md", because Read maps to `<` (`mockEvents.ts:23`). `>` is Edit and Write.
- "< heading to web" cannot occur from real events. "heading to <station>" comes only from the mock (`mockEvents.ts:76`, `:121`). A web station would show `@`, not `<`.
- "= updating the todo board" is the mock's TodoWrite text (`mockEvents.ts:48`). A real TodoWrite shows "= using TodoWrite".
- "awaiting" is real (`usePtyParser.ts:65`) and also appears in the mock (`mockEvents.ts:163`).

Other floor glyphs: animated dots while thinking (`ThoughtBubble.ts:105-108`, `:213-221`), `!` for blocked, a sparkle for success, a violet box for compacting, and an orange ring for looping (`Character.ts:826-851`).

**Two writers conflict.** While a terminal view is mounted, the hook path writes "using Bash" (`useHive.ts:497`) and the parser writes "bash npm test" (`usePtyParser.ts:126-131`) into the same `action` field. The bubble alternates between them.

### 3.6 Message envelopes

- A routed hive message emits `hive:message` with `{from, to, act, targets, needsHuman}` (`main/hive.ts:1790-1801`). `needsHuman` is `msg.to === 'human'`.
- The renderer spawns one envelope per target in `spawnHandoff` (`OfficeFloor.tsx:1618-1627`). It is skipped for self-mail, unknown endpoints, and when 16 envelopes are already in flight (`OfficeFloor.tsx:242`, `:1619`).
- **Colours by speech act** (`MessageEnvelope.ts:17-25`): request sky, query lilac, propose lemon, inform cream, agree and done mint, refuse coral. Escalation to the human is always coral (`:28`).
- **Flight:** a 14×10 px envelope, arcing 38 px up, at 230 px/s, with duration clamped to 0.8 to 2.0 s (`MessageEnvelope.ts:30-34`, `:65-66`). Arrival is a 0.34 s expanding ring (`:37`, `:130-139`).
- **Mock path:** `maybeFlyMessage` dispatches `cth:demo-handoff` with a random act (`mockEvents.ts:136-146`). The floor handles it at `OfficeFloor.tsx:1639-1643`.
- **What an envelope means:** a message was routed from one agent to another. It shows no content and does not say whether the message was read.

### 3.7 Task-board choreography

- The office has two cork boards (blockers red on the left, todo yellow on the right), an archive table, and an ask-me board (`OfficeFloor.tsx:1016-1115`, `:1134-1179`).
- Ledger changes become moves: pin, take, and archive (`OfficeFloor.tsx:1190-1199`, diffing at `:1312-1375`). A worker walks to the todo board, takes the note, carries it home, and pins it to its desk. Finishing carries the note to the archive table.
- Timing is scripted: a 900 ms beat at the board (`OfficeFloor.tsx:1264`) and a 30 s global watchdog that hard-syncs the boards (`:1288-1299`).
- The thought text for each move is hard-coded English with an emoji (`OfficeFloor.tsx:1349`, `:1352`, `:1355`, `:1358`): "pinning a new task", "grabbing my task", "filing it as done", "this one is stuck". Not translated.

The ledger decides what happens. The walk, the pause, and the thought text are scripted.

### 3.8 Fake and filler inventory

| Element | Trigger | Real or fake | Refs |
|---|---|---|---|
| Mock agent loop | no live PTY, or dev demo flag | fake | `App.tsx:210-213`, `mockEvents.ts:60-173` |
| Mock "heading to <station>" | mock loop | fake | `mockEvents.ts:76`, `:121` |
| Walking to stations | none on the real floor; `currentStation` is never read by the scene | not implemented | grep of `scene/office` |
| Cafeteria breaks and quips | random, idle or success only | canned lines | `OfficeFloor.tsx:729-794`, `cafeteriaLines.ts` |
| Coffee economy | random, after a break | internal stock, not tied to work | `OfficeFloor.tsx:439-600` |
| Errands (water, windows, dispensers, fridge, shelf, bins, cigar) | random, idle only | filler | `OfficeFloor.tsx:868-937`, `themeRegistry.ts:178-200` |
| Gossip and suck-up lines | proximity to the god | canned; one line includes the real done count | `OfficeFloor.tsx:611-624`, `:939-992`, `en.json:749-770` |
| Cheer and confetti | at least 60 s of busy time, then idle | trigger is real; effect is decorative | `Character.ts:461-481`, `OfficeFloor.tsx:1464-1467` |
| Monitor screen animation | seated | decorative | `DeskScreen.ts:64-80` |
| Steam, sink suds, window streaks, fridge flicker, paper ball | busy, or random | decorative | `OfficeFloor.tsx:474-507`, `:820-857`, `Character.ts:707-720` |
| Idle 30/30 roam and rest | idle | timer, not tied to real idleness | `Character.ts:56-57`, `:905-938` |
| Wandering | idle | random walk | `Character.ts:940-958` |
| `type` and `read` poses | never selected | dead code | `Character.ts:14`, `:107` |
| Typing pose | none | not implemented | none |

**What the screenshots show:** `docs/media/home-screen.png` (labelled v0.1) shows cafeteria quips such as "which one of you is the new guy?", "the pothole, Michael!", and "doesn't matter." Those strings are in `cafeteriaLines.ts:77`, `:130`, and `:190`. `docs/media/floor.png` (labelled v0.3.9) shows agents labelled "starting up" and "awaiting". "starting up" is set at spawn (`components/AddAgentModal.tsx:464`, `hooks/useHive.ts:1043`, `hooks/useRestoreTeam.ts:161`), and "awaiting" comes from the parser's idle rule. Both are real status text, but they are not activity.

---

## 4. Themes, seasonal decorations, day and night (Q4)

### 4.1 Theme registry

- `themeRegistry.ts:32-40` declares six theme ids: `office`, `friends`, `brooklyn99`, `siliconvalley`, `got`, `hogwarts`.
- Only two are registered (`themeRegistry.ts:290-293`): `office` and `brooklyn99`.
- Unknown ids fall back to the office in `getTheme` (`themeRegistry.ts:297-299`) and in `loadTheme` (`themeLoader.ts:58-65`).
- The picker's labels and `built` flags are in `components/OfficeThemePicker.tsx:17-23`:

| Id | Label in picker | `built` flag | What actually renders |
|---|---|---|---|
| `office` | The Office | true | the office map and the procedural cast |
| `brooklyn99` | Brooklyn Nine-Nine | true | a second map (`brooklyn99.tmj`) using the office tiles, palette, and cast. The code calls it "PLACEHOLDER ART" (`themeRegistry.ts:223-227`, `:231-233`) |
| `friends`, `siliconvalley`, `got`, `hogwarts` | Friends, Silicon Valley, Game of Thrones, Harry Potter | false | the office, plus a "not built yet" note |

Marking brooklyn99 as built is misleading, because it is a placeholder.

**Switching themes is destructive.** `applyTheme` (`OfficeThemePicker.tsx:64-81`) kills the PTY of every non-god agent (except the prep assistant), disposes its terminal, and archives the agent before it writes the new theme (`:74-78`). A confirmation modal runs first (`onSelect`, `:57-62`).

### 4.2 Seasonal decorations

- **None in the app.** A case-insensitive search for halloween, pumpkin, spooky, and jack-o returns only marketing files:
  - `tools/landing-halloween/` (`halloween.js`, `halloween.css`, `build.cjs`, `base.html`, `cast-halloween.json`). `halloween.css:2` describes it as a "Halloween dress for the munderdiffl.in story page".
  - `docs/media/office-halloween.mp4` and its poster, referenced by `docs/index.html:919` and `tools/landing-halloween/build.cjs:116-120`.
- The Halloween pass is a website skin. The office floor is not changed.

### 4.3 Day and night

- **None.** A search for night, dusk, sunset, daylight, lighting, ambient, tint, and timeOfDay in `scene/` and `store/` finds no lighting state. The only "night" in the code is the cafeteria line "I can’t do this all night." (`cafeteriaLines.ts:167`).
- The floor background is fixed at `colors.ink[900]` (`themeRegistry.ts:209`). The app's light and dark themes do not change the floor.

### 4.4 Brooklyn 99 map

- `tools/gen-b99-map.cjs` writes `assets/maps/brooklyn99.tmj` (header at `gen-b99-map.cjs:1-17`). It uses placeholder gids from the office tileset, copies desk stamps from `office.tmj`, and runs a flood-fill check that every seat, coffee stand, and spawn is reachable from the entrance before writing.
- The layout is a precinct bullpen with a glass office for the boss, a briefing room, and a break room. The theme's blurb reads "The 99th precinct bullpen".

---

## 5. Assets and licensing (Q5)

### 5.1 Inventory

| Asset | Location | Origin (as stated in repo) | Terms (as stated in repo) | Status |
|---|---|---|---|---|
| `office-tileset.png` (256×512, 16 px tiles, 512 tiles) | `assets/tilesets/` | LimeZu, "Modern Interiors - RPG Tileset [16X16]" | Complete Version licence: may edit and use in commercial projects; may not resell or distribute the asset to others; credit required, linking to limezu.itch.io | Used by both maps |
| `a5-office-floors-walls.png` (256×512) | `assets/tilesets/` | LimeZu (per `ATTRIBUTION.md`) | same as above | Atlas at gid 513 |
| `interiors.png` (256×1424) | `assets/tilesets/` | LimeZu | same as above | Atlas at gid 1025 |
| `office.tmj` (34×22) | `assets/maps/` | "vendored from shahar061/the-office" (ISC), built on LimeZu tiles | ISC for the layout source; LimeZu for tiles | Stamps copied from `tools/mapgen/original-office.tmj` |
| `brooklyn99.tmj` (36×24) | `assets/maps/` | generated by `tools/gen-b99-map.cjs` | project code; uses LimeZu gids | Placeholder |
| Press Start 2P, Inter, JetBrains Mono (latin woff2) | `assets/fonts/` | Google Fonts | SIL Open Font License 1.1; notice in `fonts/LICENSE.txt` | Compliant |
| Cast sprites and portraits | `scene/office/portraitArt.ts` (procedural) | original procedural drawings | MIT (`LICENSE-ASSETS`) | Names and blurbs are third-party characters (5.4) |
| Ported engine code (Camera, TiledMapRenderer, Character, CharacterSprite, SpriteAdapter, SeatPool, ToolBubble, pathfinding) | `scene/office/` | adapted from shahar061/the-office | ISC (per `ATTRIBUTION.md`) | No ISC notice in the repo (5.3) |

The repo contains no sprite sheets. `ATTRIBUTION.md` says the LimeZu walk sheets were deleted on 2026-08-20, once nothing referenced them.

### 5.2 LimeZu tilesets

- **Licence text** (`assets/tilesets/LIMEZUASSETS-LICENSE.txt`):
  - "YOU CAN EDIT AND USE THE ASSET IN ANY COMMERCIAL OR NON COMMERCIAL PROJECT"
  - "YOU CAN'T RESELL OR DISTRIBUTE THE ASSET TO OTHERS"
  - "CREDITS ARE REQUIRED. LINK TO https://limezu.itch.io/"
- **Credit status:**
  - The README acknowledgements have the credit (`README.md:554-568`).
  - The website has it (`docs/index.html`, `docs/terms.html`, `docs/pro/index.html`).
  - **The app does not.** A search for `itch.io` and `limezu` across `src/` finds only code comments and the asset metadata files. `ATTRIBUTION.md` says the link "must stay in the README acknowledgements, in the app's about/credits surface, and on the website". There is no about or credits surface in the app.
- **Redistribution question.** `LICENSE-ASSETS` reads the licence as permitting bundled use. The licence text says the buyer may not "distribute the asset to others", and shipping the PNGs inside a public installer is a form of distribution. I am not a lawyer. Treat the three tilesets, and the two maps that depend on them, as non-redistributable until LimeZu confirms in writing or the art is replaced. You already flagged this.

### 5.3 ISC code from shahar061/the-office

- The ISC licence requires the copyright and permission notice to appear in all copies.
- A search for "ISC License" and for the standard ISC permission sentence returns nothing in the repo. The source is credited only in code comments and in `ATTRIBUTION.md` ("project code: ISC").
- Files that say they are ported or adapted from that project, with the header line:
  - `Camera.ts:3`, `TiledMapRenderer.ts:3`, `Character.ts:8`, `CharacterSprite.ts:26`, `SpriteAdapter.ts:12`, `SeatPool.ts:4`, `ToolBubble.ts:4`, `pathfinding.ts:2`.
  - Together they are about 1,800 lines, and several are direct ports.
- **Fix:** add the ISC notice to a NOTICE or third-party file. For the successor, rewriting these modules avoids the question. They are small and well understood (BFS, a lerp camera, a sprite animator).

### 5.4 Cast, character names and show references

- The fifteen cast members are The Office characters, with display names, shirt colours, and blurbs (`cast.ts:27-43`): "World's best boss" for Michael, "Assistant (to the) RM" for Dwight, "The temp" for Ryan, and so on.
- Cafeteria lines quote the show: "that's what she said." and "I can’t do this all night." (`cafeteriaLines.ts:167`, `:190`). Gossip includes "'world's best boss' — he bought that mug himself" (`en.json`, gossip block).
- Other show names are used as theme labels and blurbs: Friends, Silicon Valley, Game of Thrones, Harry Potter (`OfficeThemePicker.tsx:19-23`).
- The map generator's header calls the office "Dunder Mifflin (The Office) styled" (`tools/mapgen/build_map.py:2`). The product name "Munder Difflin" is a close copy of the company name in the show.
- This is third-party intellectual property: character names, show references, and trade dress. It is a question for counsel, and I cannot settle it from the code. For the successor, original characters, names, and themes avoid it entirely. The procedural drawing approach is already your own and carries no third-party art.

### 5.5 Fonts

- Press Start 2P, Inter, and JetBrains Mono are bundled as latin-subset woff2 files under the SIL OFL 1.1. The notice travels with the fonts (`assets/fonts/LICENSE.txt`). Compliant.

### 5.6 Stale comments that could mislead an audit

- `portraitArt.ts:3-7` says "The in-scene walking sprites still use the LimeZu recolor in cast.ts". The code does not: `cast.ts:76-88` calls `sceneFrameBufs`, which paints procedurally.
- `components/SpritePortrait.tsx:17` describes the portrait as a "recolored LimeZu sprite".
- `CharacterSprite.ts:6` refers to "SpriteAdapter" output rows, but `SpriteAdapter.ts` has no imports (6.7).
- `SpriteAdapter.ts:4` describes `frameWidth` as "LimeZu: 16" (historical).
- `MemoryGraphPanel.tsx:19` says the graph uses "VT323/Pixelify", which the token file says were removed.

The code, `cast.ts`, `ATTRIBUTION.md`, and `LICENSE-ASSETS` agree that the cast is procedural. The stale comments are what would confuse an audit. Fix or remove them before any review.

---

## 6. UI layout, synchronisation, design tokens (Q6)

### 6.1 Layout

```
+--------------------------------------------------------------------------------+
| title bar: wordmark, version, theme toggle, settings, focus mode               |
+---------------------------------------------------------+----------------------+
| OfficeFloor (Pixi canvas)                               | right sidebar        |
|   + memory pill (MemoryPanel) bottom-left               |   AgentDetailPanel   |
|   + EMPTY FLOOR / booting overlays                      |   or CommandCenter   |
|                                                         |   (god only)         |
|                                       <- SidebarSplitter (320 to 1200 px)      |
+---------------------------------------------------------+----------------------+
| AgentStrip: roster of AgentCard buttons (drag to reorder)                      |
+--------------------------------------------------------------------------------+
Overlays: AddAgentModal, SettingsModal, QuitWarningModal,
          FullscreenTerminal (covers the app), IdePanel (full window), TaskDetailOverlay
```

References: `App.tsx` main row around lines 395 to 480 (`:402` floor, `:403` memory, `:411-424` empty floor, `:440` agent panel, `:480` roster, `:512-514` overlays). `components/SidebarSplitter.tsx:19` (min 320, max 1200) and `:28` (clamped to viewport minus 360).

### 6.2 Panels

- **Agent sidebar** (`components/AgentDetailPanel.tsx`):
  - Header (`:139-222`): 32 px portrait, editable name, status badge, project name; buttons for edit, IDE, open terminal, and kill.
  - Compact header below 440 px drops button labels (`:66-77`). The threshold and its reasoning are in comments at `:34-62`, which says the default sidebar is 420 px.
  - `AgentControlStrip` for live agents (`:234`): pause tools, stop after step, and a steer box (`components/AgentControlStrip.tsx:36-141`).
  - Tabs (`:237-287`): terminal, git, messages, traces. The terminal tab is an xterm plus a message-queue composer (`:241-274`).
- **Command Center** for the god (`components/CommandCenterPanel.tsx`): eleven tabs (`:68-82`): terminal, floor, tasks, human (ask me), triggers, trigger history, memory, graph, activity, skills, workers.
- **Roster** (`components/AgentStrip.tsx`): one `AgentCard` per agent, drag to reorder (store `reorderAgents`, `store/store.ts:241`), a restore menu for archived agents, and the doing-note poll (`:66-82`).
- **Memory pill** on the floor (`components/MemoryPanel.tsx`, 240 lines), visible in `docs/media/floor.png` as "memory · minilm".
- **Memory graph** (`components/MemoryGraphPanel.tsx`): SVG. Nodes are agents, edges are messages, and an optional topic layer comes from memory files. Positions come from `forceLayout` with 320 iterations (`components/memoryGraph/forceLayout.ts:53`), memoised (`MemoryGraphPanel.tsx:109`), and refreshed every 5 s (`:44`). It has no animation loop. The header comment says the styling is neo-pixel: square nodes, hard offset shadows (`MemoryGraphPanel.tsx:16-19`).
- **IDE** (`ide/IdePanel.tsx`, 891 lines, read for structure only): full-window overlay with a Monaco editor and diff, a file tree, markdown and image previews, and git history and compare panes. Opened from an agent header (`AgentDetailPanel.tsx:187`). Its open state pauses the floor (`OfficeFloor.tsx:210`).

### 6.3 Floor and panels: how they stay in sync

Almost all synchronisation goes through the zustand store (`store/store.ts`). The floor also calls the IPC bridge directly for the ledger and for hook events.

| Direction | Mechanism | Refs |
|---|---|---|
| Floor click selects an agent | `Character.onClick` calls `store.select` | `Character.ts:568-575`, `OfficeFloor.tsx:1404` |
| Selection moves the camera | store subscription calls `camera.nudgeToward` | `OfficeFloor.tsx:1594-1603` |
| Store agents update avatars | subscription calls `syncAgents`, which calls change-detected `applyState` | `OfficeFloor.tsx:1575-1596`, `:1449-1455` |
| Floor landmarks open panels | calendar opens triggers; boards open tasks; ask-me board opens human; clock quits | `OfficeFloor.tsx:310-316`, `:1036-1042`, `:1145-1151`, `:1128-1131` |
| Terminal output updates the store | the parser calls `updateAgent` | `hooks/usePtyParser.ts`; attached at `AgentDetailPanel.tsx:93`, `CommandCenterPanel.tsx:129`, `FullscreenTerminal.tsx:170` |
| Hook events update the store | `window.cth.onHiveHookEvent` | `hooks/useHive.ts:479` |
| Ledger updates the floor and cards | two separate pollers | `OfficeFloor.tsx:1377`, `components/AgentStrip.tsx:80` |
| Messages update envelopes and the graph | `hive:message` event | `OfficeFloor.tsx:1632-1636`, `MemoryGraphPanel.tsx` |
| Floor pauses when covered or hidden | fullscreen, IDE, or hidden document stops the ticker | `OfficeFloor.tsx:209-226` |
| Theme change rebuilds the floor | store `officeTheme` triggers a Pixi rebuild | `OfficeFloor.tsx:1774`, `OfficeThemePicker.tsx:81` |

The clock is a quit control. Clicking it calls `window.close()`, which the main process intercepts for closing-time handling (`OfficeFloor.tsx:1117-1131`).

### 6.4 Design tokens

- **Source of truth:** `design/tokens.css` (CSS custom properties). `design/tokens.ts` mirrors the values for Pixi. Both files say they must change together (`tokens.css:1`, `tokens.ts:2-3`).
- **Colour, light theme** (the root block at the top of `tokens.css`):
  - Cream 50 to 300, paper 100 and 200.
  - Ink 900 to 100, from `#1A1320` to `#D9CFE0`.
  - Accents: coral `#D96A62`, mint `#5CA97A`, sky `#4F9FAF`, lemon `#DCAB3C`, lilac `#9482D3`, peach `#D99168`. Each has a light variant.
  - Ten status colours (`--cth-status-*`): idle, thinking, working, waiting, blocked, success, ghost, compacting, looping, typing.
- **Dark theme:** `:root[data-cth-theme='dark']` at `tokens.css:125`, with a long comment above it on contrast (borders at least 3:1, body text at least 4.5:1). `design/theme.ts` stamps `data-cth-theme` on `<html>` and keeps the choice in localStorage under `cth.theme`.
- **Spacing:** base 4 px, with tokens from `space-0` (0) to `space-8` (64): 0, 4, 8, 12, 16, 24, 32, 48, 64. This matches `DESIGN.md` §5.
- **Type:** Press Start 2P for display labels (8, 12, and 16 px), Inter for UI text (13 to 16 px), JetBrains Mono for code (13 and 14 px). Line heights are integers.
- **Borders and shadows:** hairline `inset 0 0 0 1px` (`--cth-panel-border`, `tokens.css:87`). Hard shadow `3px 3px 0 rgba(26,19,32,.14)` (`:82`), and `4px 4px 0 rgba(0,0,0,.45)` in dark (`:172`).
- **Global CSS** (`design/global.css`): `image-rendering: pixelated` on every canvas, img, and svg (`:64`). Scrollbars are styled (`:88-97`). `prefers-reduced-motion` is honoured in CSS only (`:185`).
- **Hard-coded colours outside the tokens:**
  - confetti: `[0xffd93d, 0xff6b6b, 0x6bcb77, 0x4d96ff, 0xf6a6ff]` (`Character.ts:471`). `tokens.css` calls the original arcade hues "full-saturation arcade tones" and says they were retired;
  - calendar: `OfficeFloor.tsx:318-329`;
  - bubble fills and outlines: `ThoughtBubble.ts:19-21`, `ToolBubble.ts:26-28`.

### 6.5 DESIGN.md versus the code

`DESIGN.md` is the stated spec. Several rules are not what ships:

| Rule | DESIGN.md | Code |
|---|---|---|
| Floor tile grid | 32×32 (line 173) | 16×16 (`office.tmj`, `TiledMapRenderer.ts:71`) |
| Zoom | integer only, 1×, 2×, 3× (line 179) | continuous fit-to-screen (`Camera.ts:45-56`) |
| Avatar cell | 24×24, 4-frame walk, 8 fps (lines 383-385) | 18×32, drawn at 1.08×, 3 distinct frames (`portraitArt.ts:15-16`, `CharacterSprite.ts:14-24`) |
| Walk speed | 80 px/s (line 455) | 48 px/s (`Character.ts:33`) |
| Pathing | A* (line 456) | BFS (`pathfinding.ts`) |
| Stations | walk to shelf, terminal, web, and board (§9) | not implemented; the scene never reads `currentStation` |
| Attention glyph | wave hand (line 450) | not implemented (`Character.ts:15`) |
| Task-complete effect | four pixel stars, 250 ms (line 609) | 14 confetti pieces and 1.6 s of hops (`Character.ts:461-481`) |
| No bouncing | line 599 | cheer hops (`Character.ts:741`) |
| Avatars | six starter archetypes: scientist, wizard, and so on (§8.4) | The Office cast (`cast.ts`) |
| Ambient motion | "no ambient idle animations on static UI panels" (line 601) | filler is on the floor, which the doc calls the game layer, so this is arguable |
| Style | "Chunky over slick. … If a component could exist on iOS 17, it's wrong." (line 10) | the reverse of a Linear-style target |

`DESIGN.md` §2 lists its references: Animal Crossing, EarthBound, Stardew Valley, Pokémon, Mario Kart, Undertale, and Final Fantasy VI menus. That is a game-HUD vocabulary, and it conflicts with a premium productivity target.

### 6.6 Screenshots (what I saw)

- `docs/media/floor.png` (labelled v0.3.9). Read by eye, so treat the details as approximate. The floor shows several agents labelled "awaiting" and "starting up" above their avatars. Below the floor is a roster strip with status badges, small progress bars, and truncated names ("STANL..."). The right side is the Command Center with a dense grid of tab buttons.
- `docs/media/home-screen.png` (labelled v0.1). Cafeteria quips appear as bubbles. The map has black bands above and below the floor, where the fit-to-screen camera letterboxes it. Roster cards show truncated project labels ("claudeTerminalHarne…"), cut mid-word.
- Both are older than package 0.4.6. The other files in `docs/screenshots/` are `add-agent.png`, `autonomy.png`, and `memory.png`. I did not open them.

### 6.7 Dead code

- `ToolBubble` class (`ToolBubble.ts:49-220`) is never instantiated. Only `toolIcon` (`:45`) is used, by `ThoughtBubble.ts:3`. Its `showText` method (`:118`) looks like the intended "last prompt card" renderer, and it has no callers.
- `Agent.lastPrompt` is documented as "shown on the floor as a card above the seated avatar" (`store/store.ts:83`). The shipped behaviour is a thought bubble with the first words of the prompt (`OfficeFloor.tsx:151-155`). No card exists.
- `SeatPool.ts` (33 lines) and `SpriteAdapter.ts` (55 lines) have no imports.
- The `type` and `read` poses and `pendingWork` (2.2).
- `Camera.focusOn` (`Camera.ts:59-64`).
- `design/tokens.ts`: `tileSize`, `space`, `type`, `colors.world`, and `colors.paper` have no references under `src/renderer/src` (grep). Only `colors` and the accent types are imported.

---

## 7. Critique and recommendations (Q7)

### 7.1 Worth keeping

- **The ledger-to-board choreography.** A card leaves the wall with its worker and sticks to their desk, then goes to the archive. It is the strongest metaphor in the codebase.
- **Real work pre-empts filler** (`OfficeFloor.tsx:1480-1504`). Breaks, errands, and coffee always yield.
- **Pause when hidden or covered** (`OfficeFloor.tsx:209-226`).
- **Recovery design for lost WebGL contexts.** A pure policy module (`glRecovery.ts`) with a test (`test/office-gl-recovery.test.cjs`).
- **Bubble overlap resolution** (`OfficeFloor.tsx:1652-1681`).
- **Status glyph language.** `!`, sparkle, violet box, and orange ring are distinct shapes, not only colours.
- **One typeface for in-world labels.** Monospace, chosen on purpose (`Character.ts:185-190`).

### 7.2 Problems

**Clutter and duplication**

- The same state appears in up to four places: the thought bubble, the status glyph, the roster card, and the sidebar badge. "awaiting" can show on the floor, on the card, and in the sidebar at once.
- The roster card combines status, current action, context gauge, private note, task sticky, and typing badge in one small card (`AgentCard.tsx` props).
- The Command Center has eleven tabs (`CommandCenterPanel.tsx:68-82`).
- Decorative motion competes with real signal: cafeteria, gossip, errands, steam, sparkles, and the monitor animation.

**Readability and overlap**

- Thought text is 12 px and scaled by 0.5 (`ThoughtBubble.ts:22-23`, `:144-153`). On screen it is about 6 px × max(zoom, 1). At a typical zoom of 1.5 to 2.5 that is roughly 9 to 15 px, and it never gets smaller than 6 px.
- Nameplates use a 6 px world font (`Character.ts:62`) and are not counter-scaled, so they grow with zoom.
- Avatars overlap, and there is no collision avoidance (`Character.ts:70-76`, `:236-245`).
- Sideways walkers face the viewer, mirrored (`cast.ts:85`, `CharacterSprite.ts:98`).
- Bubble text is left-aligned canvas text, with no direction handling (`ThoughtBubble.ts:83-92`). The Arabic locale exists (`i18n/locales/ar.json`).
- Fit-to-screen letterboxes the map, with no pan or zoom.

**Performance**

- Always-on rendering while visible (1.4).
- Per-frame geometry rebuilds and per-agent text objects (1.5, items 2 and 3).
- Store churn from per-chunk `updateAgent` and whole-array copies (1.5, item 4).
- Three independent five-second pollers of the same ledger, plus a 30 s fetch and a 5 s graph refresh (1.5, item 5).
- A full scene rebuild on language change (1.5, item 9).
- A frame-rate-dependent camera (1.3).

**Correctness and honesty**

- Two writers share the `action` field (3.5). The bubble alternates between hook text and parser text.
- The parser is a screen scraper that the code calls a stopgap (`usePtyParser.ts:44-46`). Its four-second silence rule can mark an agent `idle` if no tool line arrives in that window (`:57-69`, `:136`, `:181`). Whether that happens in practice depends on how often the TUI repaints, which I did not measure.
- Mock data can reach real users. `startMockLoop` runs whenever no agent has a live PTY (`App.tsx:210-213`), and it animates restored agents that have no PTY (`mockEvents.ts:155`).
- Brooklyn 99 is marked `built: true` while using placeholder art (`OfficeThemePicker.tsx:20`).
- Switching theme kills every non-god agent's PTY (`OfficeThemePicker.tsx:74-78`).
- Map furniture-above is drawn under all avatars (2.5). Not verified visually.
- The store comment promises a floor card for the last prompt that does not exist (`store/store.ts:83`).

**Accessibility**

- The floor is a Pixi canvas. Its status is visible only, with no DOM equivalent, no ARIA, and no keyboard navigation. Pointer handlers are the only interaction (`Character.ts:568-575`). The roster cards are DOM buttons and are accessible.
- The scene ignores `prefers-reduced-motion`. Ambient loops run regardless. Only the CSS honours the preference (`global.css:185`).
- Floor status uses colour together with shape, which helps. Nothing on the canvas is exposed to assistive technology.

**Localisation**

- Hard-coded English: the empty-floor copy ("EMPTY FLOOR", "No agents on the floor yet.", "NO AGENT SELECTED") at `App.tsx:412-424` and `:455-475`, none of it in `en.json`; the board move thoughts (`OfficeFloor.tsx:1349-1358`); and the cafeteria lines, which are plain strings (`cafeteriaLines.ts`).
- The `i18n.language` dependency rebuilds the whole floor (`OfficeFloor.tsx:1774`). Labels should update in place.

**Tone**

- The gossip and suck-up lines mock the orchestrator and the human ("has he ever actually written code?"). That reads as charming in a demo, but a premium product where a manager might see the screen should not mock its user.

### 7.3 Recommendations for a premium successor

**Priority 0: before shipping anything**

1. **Resolve licensing.** Get a written answer from LimeZu, or replace the three tilesets. Add an in-app credits surface with the LimeZu link. Add the ISC notice, or rewrite the ported modules (about 1,800 lines).
2. **Replace The Office names, quotes, and blurbs, and the other-show themes,** with original characters and themes. The procedural drawing approach is already your own, so keep it.
3. **Keep the mock out of any path that can run beside real agents.** If a demo mode exists, make it explicit, labelled, and never an automatic fallback.
4. **One state machine as the source of truth,** driven by hook events. Keep the screen scraper as an optional hint in a separate field, so two writers never share `action`.

**Priority 1: performance**

5. **Render on change, not every tick.** Cap ambient animation at 24 to 30 fps. Pause when hidden. Make animation time-based.
6. **Pre-bake repeating effects** (steam, monitor, confetti) as frame textures, or reuse one `Graphics` per effect. Stop calling `clear()` every frame.
7. **Move text to DOM or an overlay layer.** Bubbles, nameplates, and labels in DOM give crisp text, RTL, i18n, accessibility, and no canvas text churn.
8. **Make store writes cheap.** Add equality checks to `updateAgent`, and throttle parser writes (for example to 4 Hz).
9. **One ledger subscription,** pushed over IPC, instead of three pollers.
10. **Update labels in place on language change.** Do not rebuild the scene.
11. **Limit xterm WebGL leases to the focused terminal,** so the floor is never the one evicted.
12. **Integer scaling.** Render at a fixed internal resolution and scale by integer steps, letterboxed deliberately. Or give the camera a framed region that follows activity, so the map never shows empty bands.

**Priority 2: charm without noise**

13. **Give every animation a state it represents.** Walking to the door means blocked. Sitting means working. A board move means a ledger change. Put cafeteria, gossip, and errands behind a "lively floor" setting, off by default.
14. **Implement station semantics or delete `currentStation`.** Keeping an unread field invites confusion.
15. **Honour reduced motion** with a quiet mode that stops ambient loops and keeps state glyphs.
16. **Replace the boss-mocking lines with neutral copy.**
17. **Fix furniture occlusion.** Depth-sort `furniture-above` with avatars, or move that layer below the characters.

**Priority 3: Linear and Raycast**

18. **Reduce surfaces.** One work surface with a list or board as the primary view, the floor as a live map beside it, and a single command palette (cmd-K) to jump to agents, tasks, questions, and triggers.
19. **Define a five-state vocabulary:** working, waiting on you, blocked, idle, done. Use it identically on the floor, the cards, and the sidebar, and drop the duplicated text.
20. **Rewrite the design doc.** `DESIGN.md`'s "chunky over slick" and game-HUD references conflict with a premium target. Keep pixel art to the floor only, and use a restrained neutral palette with one accent, 1 px hairlines, and 150 to 200 ms motion.
21. **Make the floor accessible.** A list of agents with names and states, and the canvas marked decorative for assistive technology.
22. **Test the event-to-visual mapping and a frame-time budget.** Today the office has one test (`test/office-gl-recovery.test.cjs`).

**Keep as design ideas:** the ledger choreography, the "real work pre-empts filler" rule, the pause-when-hidden policy, the glyph vocabulary, the one-typeface rule for in-world text, and the GL recovery policy module.

---

## 8. Not verified

- Frame rate, CPU, and GPU use. Nothing was run.
- Map furniture occlusion (2.5).
- Behaviour with more than 16 agents, or with a very large ledger.
- The LimeZu and ISC terms. Their application to shipping needs counsel.
- Whether the screen scraper's four-second rule misfires in practice (3.4).
- Whether `hiveTasks()` returns the full ledger each time. `OfficeFloor.tsx:1314-1320` reads `raw.tasks` as a whole array, so I assume it does.
- Exact visual look of the portrait recipes (`portraitArt.ts` beyond its header, frame builders, and exports).
- The IDE and Command Center internals, beyond structure.
- The sibling reports in this folder (`coordination.md`, `runtime.md`, `features.md`), which I did not read.

---

## 9. Files read

**`scene/office/` (20 files, 5,594 lines in total):**

- In full: `OfficeFloor.tsx` (1,805 lines), `Character.ts` (972), `ThoughtBubble.ts` (277), `ToolBubble.ts` (220), `MessageEnvelope.ts` (146), `TiledMapRenderer.ts` (261), `themeRegistry.ts` (299), `themeLoader.ts` (65), `cast.ts` (100), `Camera.ts` (110), `CharacterSprite.ts` (115), `DeskScreen.ts` (85), `SpriteAdapter.ts` (55), `SeatPool.ts` (33), `pathfinding.ts` (66), `glRecovery.ts` (157).
- In part: `portraitArt.ts` (591 lines; header, exports, frame builders), `cafeteriaLines.ts` (237; head and pickers).

**`store/`:** `store.ts` (types, `updateAgent`, selection-related definitions), `mockEvents.ts` (181, in full).

**`hooks/`:** `usePtyParser.ts` (184, in full); `useHive.ts` (ranges 188-228, 440-565, 1040-1045).

**`components/`:** `AgentDetailPanel.tsx` (314), `PixelPanel.tsx` (78), `MemoryPanel.tsx` (head), `AgentCard.tsx` (head), `AgentStrip.tsx` (ranges), `AgentControlStrip.tsx` (ranges), `OfficeThemePicker.tsx` (ranges), `MemoryGraphPanel.tsx` (head and render points), `CommandCenterPanel.tsx` (ranges), `SpritePortrait.tsx` (grep), `PtyTerminalView.tsx` (ranges), `terminalPool.ts` (ranges), `SidebarSplitter.tsx` (ranges), `memoryGraph/forceLayout.ts` (grep).

**Other source:** `ide/IdePanel.tsx` (imports and top-level structure only); `design/tokens.css` (173), `tokens.ts` (91), `theme.ts` (64), `global.css` (grep and selected rules); `App.tsx` (571, layout and gates); `main/hive.ts` (emit and hook-registration ranges); `preload/index.ts` (event type ranges); `i18n/locales/en.json` (office and activity keys).

**Assets and licences:** `assets/ATTRIBUTION.md`, `assets/tilesets/LIMEZUASSETS-LICENSE.txt`, `assets/fonts/LICENSE.txt`, file listing of `assets/`.

**Repo root and tooling:** `LICENSE` (MIT), `LICENSE-ASSETS`, `DESIGN.md` (sections 1, 2, 5, 8, 9, 12 and anchors), `package.json`, `package-lock.json` (pinned versions), `CHANGELOG.md` (greps), `README.md` (licence section), `tools/mapgen/build_map.py` (header), `tools/mapgen/render_map.py` (header), `tools/mapgen/original-office.tmj` (header), `tools/gen-b99-map.cjs` (header and layout), `tools/landing-halloween/` (file list and grep).

**Media:** `docs/media/floor.png`, `docs/media/home-screen.png` (viewed); names of the other files in `docs/media/` and `docs/screenshots/`.
