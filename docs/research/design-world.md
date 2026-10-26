# Codly design and World spec: Workspace UI, Living Agent World, and explainable orchestration

Research date: 2026-10-25. Scope: the visual and interaction language for Codly ("Linear + Raycast + a charming animated simulation"), the pixel-office world engine, and the explainable-orchestration inspector.

**Evidence tags used throughout**

| Tag | Meaning |
|---|---|
| **[V-src]** | Verified by reading source code cloned today (commit noted) |
| **[V-doc]** | Verified in official docs or vendor pages fetched today |
| **[V-local]** | Verified from local output on this machine (npm / crates.io / gh / fixtures in `docs/research/fixtures/`) |
| **[R]** | Reported or claimed by a project, a vendor or a third party; not independently checked |
| **[U]** | My inference or general knowledge, not verified today. Treat it as a hypothesis and spike it |

---

## 0. TL;DR: the decisions

1. **Projection.** Top-down 3/4 (oblique, Stardew/Gather style). Not isometric. 16×16 px tiles, a 16×24 px character cell, 4 facings with left mirrored from right. This is the cheapest projection to draw procedurally, z-sort, hit-test and pathfind, and it is what both shipping agent-office apps use (Pixel Agents and Munder Difflin) [V-src].
2. **Assets.** Ship **zero third-party art**. Characters are parametric sprites drawn in code, seeded from the agent id. Furniture and tiles are code-defined, palette-indexed pixel strings. Users can point Codly at a pack they own (Pixel-Agents-style manifest) as an optional "Bring your own pack", but Codly never bundles one. The reason: there is no good CC0 office pack, LimeZu and Donarg forbid redistribution, and Donarg also forbids AI/ML training [V-doc].
3. **Renderer.** Plain Canvas 2D, no PixiJS. Static floor, walls and furniture are pre-rendered once into an `OffscreenCanvas`. Sprites are blitted from `ImageBitmap` atlases. Speech bubbles and name tags are a DOM overlay, so text stays crisp, translatable and selectable by tests.
4. **Zero CPU when nothing moves.** An on-demand frame scheduler runs `requestAnimationFrame` only while something is translating. Looping poses such as typing run on step timers at 3–8 Hz. A fully static world schedules **no callbacks at all**. The loop stops on `visibilitychange` and when the World panel is off screen (`IntersectionObserver`). Pixel Agents, by contrast, runs rAF forever [V-src].
5. **Honest world.** Every animation comes from a real event and carries its provenance (event ids). There is **no random wandering by default**; "Ambient life" is an opt-in setting. State becomes visible within 250 ms through an overlay icon, even when the walk animation lags.
6. **Two-layer state machine.** An *activity* layer is driven by events and is instantaneous. A *locomotion/pose* layer is time-based. They are joined by priority, minimum dwell, and an "excursion threshold": an agent only walks to a station when the activity is still running after 400 ms and expected to last ≥ 1.2 s.
7. **Event pipeline.** The Rust core normalizes every CLI's raw stream into one `WorldActivityEvent` (schema in §9.2) and streams it over a Tauri `Channel`. The world is a pure TypeScript reducer over that log, so timeline scrub and replay come for free.
8. **Typography.** Inter Variable 4.1 (OFL, with the `opsz` axis so display sizes become Inter Display automatically) for the UI. JetBrains Mono 2.304 (OFL) for code, diffs and the terminal. Geist Pixel (OFL, geist-font v1.8.2) is optional and only for world signage. **No Berkeley Mono**: its Indie license reportedly excludes IDE and terminal apps [R].
9. **Color.** OKLCH tokens baked at build time, generated from three inputs (base, accent, contrast), the same approach Linear uses [V-doc]. Runtime relative-color syntax would need Safari 18, so it is not used. WKWebView needs only `oklch()` (Safari 15.4) and `color-mix()` (Safari 16.2) [V-local, MDN BCD]. Set `minimumSystemVersion` to `"13.0"`. The neutral is a warm gray (h 75, C 0.004). The single accent is cobalt (h 255). Contrast of the token set is checked in §5.3.
10. **Surfaces.** Hairline borders do the structure. Shadows appear only on floating layers. Radii are 6 / 8 / 12. There is one focus ring everywhere, `:focus-visible` only.
11. **Motion.** Durations of 100 / 150 / 200 / 240 ms. Ease-out-quint for entering, about 0.75× the duration for exits. A spring curve (CSS `linear()`, Safari 17.2) is used only for the palette and the inspector. `prefers-reduced-motion` is honored. Persistent chrome has no infinite CSS animation.
12. **Keyboard first.** `⌘K` opens a universal palette with `>`, `@`, `#` and `/` scopes. Navigation uses G-sequences (Linear-style). Lists take single-key actions. Approvals use `⌘↵` approve and `⌘⌫` deny. Shortcuts run on `tinykeys` 4.0.1.
13. **Inspector.** One component serves both views. Every fact carries a provenance badge: **Observed** (CLI event), **Inferred** (Codly heuristic), **Provided** (Codly retrieval put it in context, with citations), or **Model-stated** (the model's own summary). This is the core of the "explainable orchestration" feature.
14. **Cost.** Always label cost as "≈ list price, included in your plan" when a subscription CLI reports it. Claude's `result.modelUsage.*.costBasis` was `"list"` in today's fixture [V-local]. The context gauge is a **snapshot** of the current window, not a sum (a lesson recorded in Pixel Agents' notes [V-src]).

---

## 1. Prior art teardown

### 1.1 Pixel Agents (VS Code extension and `npx pixel-agents`) [V-src]

The repo `pablodelucca/pixel-agents` now redirects to **`pixel-agents-hq/pixel-agents`**. It is MIT licensed, has about 9.9k stars, was last pushed 2026-10-21, and I read commit `b1bafd0` (2026-10-16).

**Architecture.** The code is split into `core/` (contracts, AsyncAPI 3.0 protocol), `server/` (Fastify, agent runtime, Claude provider), `adapters/vscode/` and `webview-ui/` (React 19 with Canvas 2D). The game state lives in an imperative `OfficeState` class rather than React state.

**Agent detection.** Hooks mode is the default: a hook script POSTs Claude events (`SessionStart`, `PreToolUse`, `PermissionRequest`, `Stop`, …) to the local server. When hooks are unavailable it falls back to heuristic polling of `~/.claude/projects/**/*.jsonl` every 500 ms. Two timers handle the heuristic case: a permission timer of 7 s and a text-idle timer of 5 s. "Tool done" messages are delayed 300 ms so React batching cannot hide brief active states.

**Character FSM** (`webview-ui/src/office/engine/characters.ts`) has only three states, `TYPE | IDLE | WALK`:
- **Active** means pathfind to the seat, then play a typing or reading animation chosen by tool type. The reading tools come from the provider through a `providerCapabilities` message: `Read`, `Grep`, `Glob`, `WebFetch`.
- **Inactive** means sit for a while, then wander randomly. A seat-rest lasts 120–240 s. Wander pauses last 2–20 s, with 3–6 moves before returning to the seat.

**Pathfinding.** BFS on a 4-connected grid (`findPath`), no diagonals. Chair tiles are blocked for everyone except the agent that owns the seat.

**Sprites.**
- Characters are 6 pre-colored PNGs of 112×96: 7 frames × 16 px wide by 3 direction rows × 32 px tall (rows: down, up, right). Frame order is `walk1, walk2, walk3, type1, type2, read1, read2`. Left is the right row flipped at runtime.
- When there are more than 6 agents, a hue shift of 45–315° is applied.
- Credits say the characters are based on JIK-A-4 "Metro City", which is CC0 [V-doc, itch page].
- Furniture uses per-item `manifest.json` files with rotation groups, on/off state groups and `backgroundTiles`. Monitors auto-switch to their ON sprite when an active agent faces them.

**Rendering rules** [V-src, CLAUDE.md]:
- Zoom is an integer number of device pixels per sprite pixel (1–10). There is no `ctx.scale(dpr)`. Default zoom is `Math.round(2 * devicePixelRatio)`.
- Everything is z-sorted by Y. A character's sort key is `y + TILE_SIZE/2 + 0.5`. A back-facing chair uses `(row+1)*16 + 1` so the chair back draws over the character.
- Sitting shifts the character down 6 px.
- Camera follow uses a lerp of 0.1 per frame, which depends on frame rate.

**Bubbles.** A permission bubble ("…", amber) stays until it is resolved. A waiting bubble (green check) auto-fades after 2 s. Bubbles are pixel sprites stored as JSON palette plus pixel rows. Spawn and despawn use a 0.3 s "Matrix rain" effect.

**Game loop.** `startGameLoop` calls `requestAnimationFrame` every frame, forever. It sets `imageSmoothingEnabled = false` each frame and clamps `dt` to 0.1 s. **It never pauses.** Codly should do better (§9.8).

**Constants worth reusing as starting values:**

| Constant | Value |
|---|---|
| `TILE_SIZE` | 16 |
| `WALK_SPEED_PX_PER_SEC` | 48 |
| `WALK_FRAME_DURATION_SEC` | 0.15 |
| `TYPE_FRAME_DURATION_SEC` | 0.3 |
| `BUBBLE_FADE_DURATION_SEC` | 0.5 |
| `ZOOM_MIN` / `ZOOM_MAX` | 1 / 10 |
| Max layout | 64×64 |

**UI chrome.** The whole UI is pixel-styled: FS Pixel Sans, `border-radius: 0`, 2 px borders, hard offset shadows `2px 2px 0`. Custom ESLint rules enforce it.

**Lessons to carry over:**
1. Classify tools from a provider-supplied taxonomy, never from hard-coded names.
2. The context gauge must be a snapshot: newest turn `input + cache_creation + cache_read + output`, measured against a provider-declared window.
3. Hook installers must never rewrite configuration they did not author.

### 1.2 Munder Difflin (munderdiffl.in) [V-src unless noted]

The repo is `chaitanyagiri/munder-difflin`, now **`HarnessMD/munder-difflin`**. The code is MIT, it has about 9.1k stars, was pushed 2026-10-25, and I read commit `d3460de`. `package.json` is at version 0.4.6. It is built on Electron 32, React 18 and **PixiJS 8.5** (`pixi.js ^8.5.1`), with xterm 5.5 and better-sqlite3.

**Art licensing.**
- `LICENSE-ASSETS` and `assets/ATTRIBUTION.md` say the tilesets are **LimeZu "Modern Interiors" (Complete Version licence, purchased 2026-08-20)** and are vendored in the open-source repo. Their reading is "may be edited and used… may not be resold… credits required".
- **LimeZu's own page says you "can't … distribute the asset to others"** [V-doc]. Shipping the PNGs in a public repo is at best a gray area. Codly should not copy this pattern.
- The office cast is **procedurally drawn** in `scene/office/portraitArt.ts`, which is 591 lines and MIT. Each character is a recipe layering skin → clothing → face → facial hair → hairstyle → glasses on an 18×28 bust, plus an 18×32 scene sprite with legs. It uses 4 skin ramps (hi/base/shade/line) and about 9 hairstyle functions. The walk uses 3 phases (stand, step-L, step-R) facing front and back. The file comment notes that these replaced LimeZu recolors on 2026-08-20.
- **This is direct evidence that procedurally generated characters are viable and shippable.**

**`ToolBubble.ts`.**
- The text shape is `"<icon> <target>"` with ASCII icons: `Read '<'`, `Edit/Write '>'`, `Bash '$'`, `Grep/Glob '?'`, `WebFetch/WebSearch '@'`, `TodoWrite '='`, `MCP '*'`.
- Timings: fade-in 0.15 s, linger 2.0 s, fade-out 0.3 s. Maximum width is 140 px and text is capped at 150 characters.
- Text is rendered at 2× and scaled to 0.5 for crispness. Bubble positions are rounded "to stop shimmer while walking", which is a real gotcha.

**`MessageEnvelope.ts`.**
- A pixel envelope flies sender → recipient desk along an arc: lift 38 px, speed 230 px/s, duration clamped to 0.8–2.0 s, plus a 0.34 s arrival burst.
- It is tinted by speech act (`request / inform / propose / query / agree / refuse / done`).

**Other details.**
- Pathfinding is BFS, ported verbatim from `shahar061/the-office` (ISC, 17 stars).
- `glRecovery.ts` (157 lines) exists to survive WebGL context loss, which is one cost of choosing PixiJS/WebGL.
- `DESIGN.md` describes the whole UI as "Animal Crossing × Earthbound × SNES menu". Everything is pixel-snapped, there is no spring physics, hover is instant, the drawer and modal use 200 ms `cubic-bezier(.2,.8,.2,1)`, sprites walk at 8 fps with a ±1 px sine bob, and the sparkle lasts 250 ms.

**Website claims [R].** 12 CLIs supported. A "Classic" pixel-office mode is free, while **Pro replaces it** with a sidebar workspace at $150/yr after 2026-10-24.

**Takeaway.** Munder makes users choose between "game" and "work" views. Codly's differentiation is to keep both views live, synchronized, and explainable.

### 1.3 Projection study

| Option | Examples | Grid math | Art cost | Z-sort | Verdict |
|---|---|---|---|---|---|
| **Top-down 3/4 (oblique)** | Stardew Valley (16 px tiles [U]), Gather (**32×32 px tiles, max map 100×100 tiles** [V-doc]), Pixel Agents, Munder | screen = grid × tile | 3 facings, left mirrored | by foot Y | **Choose** |
| Isometric 2:1 | Habbo [U] | diamond transform plus inverse for picking | 4 diagonal facings and taller furniture variants | by (x+y) with multi-tile objects split | Too costly for procedural art; harder hit testing |
| Pure top-down | roguelikes | trivial | low charm (heads only) | trivial | Reads as a dashboard, not a world |

---

## 2. Assets: license matrix and the recommendation

| Pack | License | What's in it | Fit for Codly |
|---|---|---|---|
| Kenney (all asset pages) | **CC0, attribution not required** [V-doc kenney.nl/support] | Roguelike RPG pack: 16×16, about 1,700 files, 2015 [V-doc]. Roguelike Indoors: 480 sprites, 2015 [V-doc]. Roguelike Characters: about 450 files, 2015 [V-doc] (static layered, no walk cycles [U]) | Legally perfect but stylistically dated, with almost no office items (no PCs or servers) |
| JIK-A-4 "Metro City" characters | **CC0** [V-doc itch] | 3 base models, 4-direction run, outfits and hair (hair animated only facing front) | Fine as inspiration; incomplete animations |
| LimeZu Modern Interiors / Modern Office | Paid ($1.50 / $2.50 minimum). Use in projects is allowed. **Cannot resell or distribute to others; credit required** [V-doc] | Huge 16/32/48 px sets and a character generator | **Do not bundle.** Optional user-supplied pack only |
| Donarg / RetroPi "Office Tileset" (recommended by Pixel Agents' docs) | Paid ($2+). **Cannot resell or distribute; no AI/ML training; no NFTs; credit required** [V-doc] | Desks, PCs, laptops, printers, whiteboards, 17 floors | **Do not bundle.** User-supplied only |
| AdricCustoms "Little Bits: Office" | **No formal license field**, only a comment "Use it for whatever you like" [V-doc] | 16×16 cubicles and desks | Legally ambiguous; avoid |
| rixitic "2D Sidescroller Office" | CC BY-SA 4.0 [R search result] | Side view | Share-alike and wrong projection; avoid |
| OpenGameArt LPC family | CC-BY-SA 3.0 / GPL 3 [U] | Characters and furniture | Share-alike obligations; avoid |

**Recommendation: procedural art, zero bundled third-party assets.** The reasons, in order:
1. No CC0 office pack has good modern office furniture.
2. Everything good is paid and non-redistributable.
3. Agent identity should be *generated*, so N agents get N stable, distinct looks.
4. Palette-indexed art lets the world follow the app's light and dark themes.
5. Munder has already proven procedural characters ship [V-src].

### 2.1 Procedural character spec

```
Cell: 16×24 art px, anchor = feet center (8, 23). Draw order (back → front):
  shadow (1-px ellipse, 30% black) → legs/pants → shoes → torso/top → back arm
  → head (skin) → face (eyes, mouth) → hair (style × facing) → front arm → accessory
  → auto-outline pass (1 px, from alpha mask; outline color = darkest shade of the
    neighbouring layer, "selective outline").
Head 10×9 (rows 2–10), torso 8×7 (rows 11–17), legs 6 (rows 18–23). Chibi proportions
read well at 2–4× CSS zoom.
Facings: down, up, side (left = mirror of side at draw time).
```

**Palettes.** Every ramp is generated in OKLCH and converted to sRGB once at atlas build.

| Slot | Choices | Ramp |
|---|---|---|
| Skin | 6 ramps from light to deep (hue 45–70, L 0.86→0.42) | hi / base / shade / line |
| Hair | 10 colors (black, brown ×3, auburn, blond ×2, gray, white, one fun color: teal or pink) | hi / base / shade |
| Top | **Agent identity color** `--id-n` (§5) | base L, shade L−0.10, highlight L+0.08, C × 0.9 |
| Pants | 4 neutrals (charcoal, navy, khaki, denim) | base / shade |
| Hairstyle | short, side-part, curly, bun, long, buzz, spiky, bob, bald | per-facing functions |
| Top cut | tee, hoodie (hood rim when facing up), shirt with collar, cardigan, overalls | |
| Accessory (≤ 1) | glasses, headphones, beanie, cap, lanyard | |

**Determinism.** `recipe = fromSeed(fnv1a(agentStableId))` using `mulberry32`. The user can re-roll or edit the look in the inspector; edits are persisted in SQLite. **Never use provider logos** (Claude, OpenAI, Google, Cursor) on sprites. Show the provider as text on the desk nameplate and as a lanyard color.

**Poses and frames.** Frame counts and timings below are starting values; tune by eye.

| Pose | Frames | Timing | Notes |
|---|---|---|---|
| `stand` | 1 | – | idle, standing |
| `walk` | 4 (contact-L, pass, contact-R, pass) | 125 ms/frame (8 fps); Pixel Agents uses 150 ms [V-src] | +1 px body bob on pass frames |
| `sit` | 1 | – | the shift-down-when-seated offset follows Pixel Agents (6 px) [V-src] |
| `sit_type` | 2 (hands alternate) | 300 ms (Pixel Agents value [V-src]) | monitor glow flickers in sync |
| `sit_read` | 2 (paper in hands, eye shift) | 600 ms | |
| `sit_think` | 1 + blink | blink every 4–6 s **only while the activity is live** | hand to chin |
| `raise_hand` | 2 | 500 ms | approval and question |
| `celebrate` | 3 (arms up, 1 px hop) | 120 ms × 2 loops | tests pass, merge |
| `slump` | 1 | – | blocked or error |
| `sleep` | 1 | – | rate-limited or long idle, plus a "zZ" overlay |
| `carry` | walk variant + prop layer | 125 ms | paper, package, envelope |
| `point` | 1 | – | delegating to a helper |
| `phone` | 2 | 500 ms | MCP call ("asking linear…") |

**Atlas build.** All frames for all facings go into one `ImageData` per recipe, then `createImageBitmap()`. That is about 3 facings × 24 frames × 16×24 px, roughly 28 KB of RGBA per agent. Keep a 1-bit alpha mask per frame for pixel-accurate hit testing.

```ts
// world/sprites/character.ts (sketch)
export interface Recipe { skin: number; hair: HairStyle; hairColor: number; top: TopCut;
  topHue: number /* id-n hue */; pants: number; acc?: Accessory }
export function recipeFromSeed(id: string): Recipe {
  const rnd = mulberry32(fnv1a(id));
  return { skin: pick(rnd, 6), hair: pickOf(rnd, HAIR_STYLES), hairColor: pick(rnd, 10),
           top: pickOf(rnd, TOPS), topHue: ID_HUES[fnv1a(id) % ID_HUES.length],
           pants: pick(rnd, 4), acc: rnd() < 0.35 ? pickOf(rnd, ACCESSORIES) : undefined };
}
export async function buildAtlas(r: Recipe): Promise<CharacterAtlas> {
  const img = new ImageData(16 * FRAMES.length, 24 * 3);       // rows: down, up, side
  FRAMES.forEach((f, i) => FACINGS.forEach((dir, row) =>
    drawFrame(img, i * 16, row * 24, r, dir, f.pose, f.index)));  // pure pixel ops
  return { bitmap: await createImageBitmap(img), masks: buildMasks(img), frames: FRAME_INDEX };
}
```

### 2.2 Furniture and tiles as code (a pixel DSL)

This uses the same idea as Pixel Agents' bubble JSON (`{palette, pixels}`) [V-src]. Palette keys resolve to **world theme tokens**, so a night palette is just a palette swap.

```ts
export const SERVER_RACK: PixelArt = {
  w: 16, h: 32, footprint: [1, 1], backgroundRows: 1, // top row overlaps the wall
  palette: { o: 'w.outline', m: 'w.metal', M: 'w.metal.hi', d: 'w.metal.lo', g: 'w.led.on', '.': null },
  frames: { idle: [/* 32 strings of 16 chars */], busy: [/* LED pattern A */, /* LED pattern B */] },
};
```

**Catalog for v1** (about 25 items, 1–2 days of pixel work): desk 32×16 plus front, chair in 4 orientations, monitor (off, on, and an on-code animation of 2 frames), laptop, bookshelf 16×32, filing cabinet, server rack (idle and busy), terminal kiosk, whiteboard 32×32 with up to 8 sticky-note slots, mailbox with flag up and down, coffee machine, plant ×2, door (closed and open), window (day and night), rug, wall with a 4-bit autotile mask (N=1, E=2, S=4, W=8, as Pixel Agents does [V-src]), and floors (wood, carpet, tile) with seeded dithering.

**Optional BYO pack.** Read a user folder with the same manifest schema as Pixel Agents (`docs/external-assets.md`: `id, type: asset|group, footprintW/H, rotationScheme, backgroundTiles, canPlaceOnSurfaces`). Store only the path. Show a "You're responsible for this pack's license" note.

---

## 3. Design language: what Linear, Raycast and Vercel actually do

### 3.1 Verified facts

**Linear, "How we redesigned the Linear UI (part II)", 2024-03-28** [V-doc]:
- Moved theme generation from HSL to **LCH**.
- A theme went from **98 variables to 3: base color, accent color, contrast**. The contrast variable enables high-contrast themes.
- LCH is used to derive surface elevations (background, panels, dialogs, modals).
- They reduced how much chrome blue feeds into the calculations, for a more neutral look.
- Text and neutral icons were made darker in light mode and lighter in dark mode.
- **Inter Display** is used for headings and **Inter** for everything else.

**Linear, "Behind the latest design refresh", 2026-03-12** [V-doc]:
- Two principles: "Don't compete for attention you haven't earned" and "Structure should be felt not seen".
- The sidebar was dimmed a few notches, with smaller icons and muted inactive text.
- Tabs became compact with rounded corners; the first tabs are icon-only pills.
- There are fewer separators, with softened contrast.
- Default palettes moved from cool blue-leaning gray to a **warmer gray**, "crisp but less saturated" (too warm looks muddy).
- Token values are exported as JSON.

**Linear keyboard** [V-doc, scattered docs]: `Cmd/Ctrl K` command menu on a selection; **G-then-X navigation sequences** (`G I` inbox, `G M` my issues); `O`-sequences to open things; `[` collapses the sidebar.

**Vercel Geist** [V-doc]:
- Colors: 10 scales × 10 steps (100–1000) with fixed semantics. 100 = default background, 200 = hover background, 300 = active background, 400 = default border, 500 = hover border, 600 = active border, 700 = high-contrast background, 800 = hover high-contrast, **900 = secondary text, 1000 = primary text**. `background-100/200` are separate, and `gray-alpha` is reserved for overlays and shadows. P3 is used where supported.
- Materials: base and small at **6 px** radius, medium and large at **12 px**, tooltip 6 px (the only one with a stem), menu 12 px, modal 12 px, fullscreen 16 px.
- Typography classes: `text-label-14` is the most common menu style; `label-13` is for secondary text and uses tabular figures; `copy-14` is the most common multi-line style; `-mono` variants exist.

**Vercel Web Interface Guidelines** (MIT, pushed 2026-10-05) [V-doc]. These are adopted wholesale as Codly's checklist. Highlights:
- `:focus-visible` rings.
- Hit targets ≥ 24 px.
- Spinners get a show-delay of about 150–300 ms and a minimum visible time of 300–500 ms.
- Use the ellipsis character `…` for follow-up and loading labels.
- Confirm destructive actions or offer Undo.
- **Honor `prefers-reduced-motion`.**
- Order of preference: CSS, then the Web Animations API, then JS.
- Animate only `transform` and `opacity`; never `transition: all`.
- Animations must be interruptible.
- Layered shadows; borders combined with shadows.
- Nested radii must be concentric.
- **Prefer APCA** over WCAG 2.
- Interactions increase contrast.
- Set `color-scheme: dark`.
- Use tabular numbers.
- Don't rely on color alone.
- Error messages guide the exit.
- Use `aria-live="polite"` for toasts.
- Tooltip timing: delay the first; peers in the same group get no delay.

**Raycast developer API** [V-doc]:
- Primary action is `↵` and secondary is `⌘↵` in List, Grid and Detail. In Form they are `⌘↵` and `⌘⇧↵`.
- Toast styles are `Animated` (a running process, updated in place), `Success` and `Failure`. A toast can have a primary and a secondary action, each with a shortcut.
- If the window is closed, a toast falls back to a HUD.
- Common shortcuts: Copy `⌘⇧C`, Copy name `⌘⌥C`, Copy path `⌘⌃C`, Duplicate `⌘D`, Edit `⌘E`, New `⌘N`, Open `⌘O`, Open with `⌘⇧O`, Pin `⌘.`, Refresh `⌘R`, Remove `⌃X`, Remove all `⌃⇧X`, Save `⌘S`, Quick Look `⌘Y`, Move up/down `⌘⌥↑`/`⌘⌥↓`.

The ui-ux-pro MCP was consulted. It returned generic patterns (auto-dismiss toasts after 3–5 s, visible focus, empty states with an action) plus "dark OLED / cyberpunk neon glow" styles. Those styles are **explicitly rejected** as the generic AI-dashboard look.

### 3.2 Anti-patterns: the "generic AI dashboard" look we avoid

- Purple→blue gradients
- Glassmorphism blur on every panel
- Neon glow text shadows
- A ✨ sparkle as the AI icon
- A grid of KPI cards with heavy shadows
- A chat-only UI
- An "AI orb" avatar
- Emoji in system copy
- Skeleton shimmer everywhere
- Bouncing UI

In Codly, **personality lives in the world, the pixel portraits and the copy. The chrome stays quiet.**

### 3.3 Typography decisions

| Font | Version (verified today) | License | Use in Codly |
|---|---|---|---|
| **Inter Variable** (`@fontsource-variable/inter` 5.3.0, ships `opsz.css`) | Inter 4.1 (2024-11-16, latest) [V-local gh] | SIL OFL 1.1, no Reserved Font Name [V-doc] | All UI. Load `opsz.css` and `font-optical-sizing: auto` so ≥ 20 px renders as Inter Display (Linear's split) |
| **JetBrains Mono** (`@fontsource-variable/jetbrains-mono` 5.3.0) | 2.304 (2023-01-14, latest) [V-local] | OFL 1.1 [V-local] | Code viewer, diffs, terminal (xterm), inline code, ids |
| Geist / Geist Mono (`geist` 1.8.2) | v1.8.2 (2026-10-21) [V-local] | OFL 1.1 | Not used, to avoid two sans faces. Note that Geist Mono 1.7 moved coding ligatures to `ss11` [V-local release notes] |
| **Geist Pixel** (in geist-font `fonts/GeistPixel`, 5 variants: Square, Grid, Circle, Triangle, Line) | v1.8.x [V-local] | OFL 1.1 | Optional, for world signage (room labels, desk nameplates) at integer sizes. It is designed as a display font [V-doc readme] |
| Berkeley Mono | – | Commercial. Indie tier is about $225; **IDEs, terminals and editors generally not allowed** [R search snippet; vendor page returned 403] | **Do not use** |

Inter's old "dynamic metrics" page (rsms.me/inter/dynmetrics) now returns 404 [V-doc]. With Inter 4 and `opsz`, do not apply the legacy tracking formula. Use `letter-spacing: -0.01em` at ≥ 20 px and `-0.02em` at ≥ 32 px only [U].

Font features: `font-feature-settings: "cv05", "cv08"` gives a tailed `l` and a serifed `I` [U, check against the Inter 4.1 feature list]. This disambiguates paths and identifiers. Use `font-variant-numeric: tabular-nums` for every number that updates live (tokens, cost, timers).

---

## 4. Codly visual system

**Density.**
- Base UI text is 13/20. List rows are 28 px; sidebar rows are 28 px. Controls are 28 px (compact) or 32 px (default). The title bar is 40 px. The tab bar is 36 px.
- The spacing grid is 4 px.
- Defaults are compact; a "Comfortable" toggle adds 4 px to rows.

**Borders vs shadows.**
- Structure comes from 1 px hairlines: `--border-1` inside panels, `--border-2` around inputs.
- Shadows exist only on floating layers (menus, popovers, palette, dialogs, toasts) and are always paired with a 1 px border ring.
- In dark mode, shadows are nearly invisible. Floating layers instead get a lighter surface (`--bg-3`) and an inner top highlight.

**Focus.** `outline: 2px solid var(--focus); outline-offset: 2px` on `:focus-visible`. Inside dense lists and the canvas, use `outline-offset: -2px`. The focus color meets ≥ 3:1 against every surface (§5.3).

**Selection.** The active row gets `--fill-3` plus a 2 px accent bar on the left in list contexts. The accent fill (`--accent-soft`) is used only for *current* selection, never for hover.

**Command palette (`⌘K`).**
- 640 px wide, top at 18vh, radius 12, `--bg-3`.
- An input row of 48 px with scope chips. Results are 36 px rows: icon, title, subtitle in `--text-3`, and a right-aligned shortcut `Kbd`.
- Section headers are 11 px uppercase with +0.04em tracking.
- Scopes: `>` commands, `@` agents, `#` tasks, `/` files, `?` help. Typing a scope prefix converts it to a chip.
- `↵` runs the primary action, `⌘↵` the secondary (Raycast parity). `⌘K` inside the palette opens an action submenu for the highlighted row (Raycast-style action panel). `Esc` pops the scope, then closes.
- Fuzzy matching uses `@leeoniya/ufuzzy` 1.0.19 (MIT) [V-local]. Rank recent and frequent items first.
- An "Ask Codly…" fallback row is always last, so a query never dead-ends.

**Toasts** (bottom-right, max 3 stacked, 360 px):
- `progress` behaves like Raycast's Animated style and updates in place.
- `success` auto-dismisses after 4 s; `info` after 6 s.
- `error` persists, with actions "View log" and "Copy error".
- Toasts are polite `aria-live`. Hovering pauses dismissal.
- **Approvals are never toast-only.** A toast for an approval is just a pointer into the Inbox. The real request lives in the inbox, the world, the agent card, the dock badge, and a native notification sent from Rust.

**Empty states with personality.** Each empty state is one sentence of character plus one action and its shortcut. No illustrations except the world.

| Surface | Copy |
|---|---|
| World, no agents | "The office is quiet. Start a task (⌘N) and someone will grab a desk." |
| Approvals inbox | "Nobody's waiting on you. Nice." |
| Board column "Blocked" | "Nothing's stuck. Keep it that way." |
| Timeline before the first run | "Every tool call lands here as it happens." |
| Search, no results | "No matches for “{q}”. Try `/` for files or ask Codly." |
| Agent finished with no changes | "Done, and no files changed. Here's what Ada looked at." |

**Copy rules** (Vercel guidelines adapted [V-doc]):
- Active voice; `…` (not `...`); numerals; a space before units (`12 kB`).
- Errors say how to get out ("Claude Code isn't signed in. Run `claude` once in a terminal, then retry.").
- Sentence case for UI, not Title Case. This is a deliberate deviation from Vercel's own preference.
- No first-person "I" from Codly in system UI. Agents may speak in first person **only inside world bubbles**, and only to restate real events ("Reading app.ts…").

---

## 5. Design tokens

### 5.1 CSS (paste-ready)

All values are OKLCH and baked at build time. The hex values are the sRGB conversions computed today with Ottosson's matrices. Every listed color is inside sRGB, except the dark `--accent-hover` which was clamped by reducing chroma.

```css
/* tokens.css — Codly v1. Generated from: base={h:75,c:0.004}, accent={h:255}, contrast=standard */
@import "@fontsource-variable/inter/opsz.css";
@import "@fontsource-variable/jetbrains-mono/wght.css";

:root {
  /* ---------- type ---------- */
  --font-ui: "Inter Variable", -apple-system, BlinkMacSystemFont, system-ui, sans-serif;
  --font-mono: "JetBrains Mono Variable", ui-monospace, "SF Mono", Menlo, monospace;
  --font-features: "cv05" 1, "cv08" 1;            /* verify against Inter 4.1 */
  --text-11: 11px; --lh-11: 16px;   /* kbd, captions, section headers (uppercase +0.04em) */
  --text-12: 12px; --lh-12: 16px;   /* metadata, chips, timestamps */
  --text-13: 13px; --lh-13: 20px;   /* base UI */
  --text-14: 14px; --lh-14: 20px;   /* chat body, composer, inputs */
  --text-16: 16px; --lh-16: 24px;   /* panel titles */
  --text-20: 20px; --lh-20: 28px;   /* view titles (opsz → Display) */
  --text-24: 24px; --lh-24: 32px;   /* empty-state headline */
  --text-32: 32px; --lh-32: 40px;   /* onboarding only */
  --mono-12: 12px; --lh-mono-12: 18px;   /* code, diffs */
  --mono-13: 13px;                        /* terminal default (user adjustable) */
  --weight-regular: 400; --weight-medium: 500; --weight-semibold: 600;

  /* ---------- space & size (4-px grid) ---------- */
  --sp-0_5: 2px; --sp-1: 4px; --sp-1_5: 6px; --sp-2: 8px; --sp-3: 12px; --sp-4: 16px;
  --sp-5: 20px; --sp-6: 24px; --sp-8: 32px; --sp-10: 40px; --sp-12: 48px;
  --row-sm: 24px; --row: 28px; --control: 32px; --control-lg: 36px;
  --titlebar-h: 40px; --tabbar-h: 36px; --sidebar-w: 232px; --inspector-w: 384px;

  /* ---------- radii (concentric: child ≤ parent) ---------- */
  --r-xs: 3px; --r-sm: 4px; --r-md: 6px; --r-lg: 8px; --r-xl: 12px; --r-2xl: 16px; --r-full: 999px;

  /* ---------- motion ---------- */
  --dur-0: 0ms;      /* hover colour on dense lists */
  --dur-1: 100ms;    /* press, tooltip out, small fades */
  --dur-2: 150ms;    /* hover bg on controls, menu out */
  --dur-3: 200ms;    /* menus/popovers/toasts in */
  --dur-4: 240ms;    /* dialogs, inspector slide, view switch */
  --dur-spring: 320ms; /* palette + inspector only; visually lands at ~150ms */
  --ease-out: cubic-bezier(0.23, 1, 0.32, 1);          /* ease-out-quint: enters */
  --ease-in-out: cubic-bezier(0.65, 0, 0.35, 1);       /* on-screen moves */
  --ease-exit: cubic-bezier(0.4, 0, 1, 1);             /* exits at ~0.75× duration */
  /* damped spring k=400 c=30 (ζ≈0.75, ≈2.8% overshoot), sampled to 24 stops; Safari ≥17.2 */
  --ease-spring: linear(0, 0.05, 0.169, 0.316, 0.467, 0.607, 0.726, 0.822, 0.896, 0.95, 0.986,
    1.009, 1.022, 1.027, 1.028, 1.026, 1.022, 1.018, 1.014, 1.01, 1.007, 1.004, 1.002, 1);

  /* ---------- z ---------- */
  --z-sticky: 10; --z-panel: 20; --z-overlay: 30; --z-popover: 40; --z-toast: 50;
  --z-modal: 60; --z-palette: 70; --z-tooltip: 80;
}

/* ===================== DARK (default) ===================== */
:root, [data-theme="dark"] {
  color-scheme: dark;
  /* surfaces */
  --bg-0: oklch(0.155 0.004 75);  /* #0d0c0a window, sidebar, title bar */
  --bg-1: oklch(0.178 0.004 75);  /* #12110f content canvas */
  --bg-2: oklch(0.205 0.005 75);  /* #191715 raised: cards, inspector, composer */
  --bg-3: oklch(0.230 0.005 75);  /* #1e1d1a floating: menus, palette, dialogs */
  /* neutral scale (Geist-style semantics) */
  --n-100: oklch(0.215 0.004 75); /* #1b1917 */
  --n-200: oklch(0.245 0.005 75); /* #22201e */
  --n-300: oklch(0.275 0.005 75); /* #292725 */
  --n-400: oklch(0.300 0.005 75); /* #2f2d2b */
  --n-500: oklch(0.360 0.006 75); /* #3f3d3a */
  --n-600: oklch(0.440 0.006 75); /* #54524f */
  --n-700: oklch(0.640 0.006 75); /* #8e8c88 */
  --n-800: oklch(0.700 0.006 75); /* #a19e9a */
  --n-900: oklch(0.780 0.006 75); /* #bab7b3 */
  --n-1000: oklch(0.950 0.004 75);/* #f0eeeb */
  /* semantic aliases — components use ONLY these */
  --fill-1: var(--n-100); --fill-2: var(--n-200); --fill-3: var(--n-300);  /* control rest/hover/active */
  --border-1: var(--n-400); --border-2: var(--n-500); --border-3: var(--n-600);
  --text-1: var(--n-1000); --text-2: var(--n-900); --text-3: var(--n-700); --icon: var(--n-800);
  /* accent (cobalt) */
  --accent: oklch(0.555 0.180 255);        /* #0671d9 primary button bg; white text 4.76:1 */
  --accent-hover: oklch(0.515 0.165 255);  /* darker on hover: interactions increase contrast */
  --accent-text: oklch(0.780 0.105 255);   /* #89bafa links, active tab label */
  --accent-soft: color-mix(in oklch, var(--accent) 18%, transparent);
  --on-accent: oklch(0.995 0 0);
  --focus: oklch(0.720 0.145 255);         /* #62a7fd ≥7:1 on bg-1/bg-2 */
  /* status (fills + text); never colour-only: always icon + label */
  --success: oklch(0.760 0.150 155);  /* #52cd86 */
  --warning: oklch(0.830 0.150 80);   /* #fabb41  approvals / waiting */
  --danger:  oklch(0.700 0.180 25);   /* #fa6863 */
  --running: var(--accent-text);      /* "live" = the one accent */
  --warning-text: var(--warning); --success-text: var(--success); --danger-text: var(--danger);
  /* agent identity (8 equal-weight hues, L .74 C .13) — shirts, avatar rings, timeline lanes */
  --id-1: oklch(0.74 0.13 20);  --id-2: oklch(0.74 0.13 60);  --id-3: oklch(0.74 0.13 105);
  --id-4: oklch(0.74 0.13 150); --id-5: oklch(0.74 0.11 195); --id-6: oklch(0.74 0.13 240);
  --id-7: oklch(0.74 0.13 285); --id-8: oklch(0.74 0.13 330);
  /* elevation: border ring + layered shadow + top highlight */
  --ring: 0 0 0 1px oklch(1 0 0 / 0.07);
  --hl: inset 0 1px 0 oklch(1 0 0 / 0.04);
  --shadow-1: var(--ring);                 /* dark: raised = border ring only, no blur */
  --shadow-pop: var(--ring), var(--hl), 0 4px 8px -2px oklch(0 0 0 / 0.45), 0 12px 24px -6px oklch(0 0 0 / 0.5);
  --shadow-modal: var(--ring), var(--hl), 0 8px 16px -4px oklch(0 0 0 / 0.5), 0 24px 48px -12px oklch(0 0 0 / 0.6);
  --scrim: oklch(0 0 0 / 0.5);
}

/* ===================== LIGHT ===================== */
[data-theme="light"] {
  color-scheme: light;
  --bg-0: oklch(0.972 0.003 75);  /* #f7f5f4 */
  --bg-1: oklch(0.993 0.002 75);  /* #fdfdfb */
  --bg-2: oklch(1 0 0);           /* #ffffff */
  --bg-3: oklch(1 0 0);           /* #ffffff (+shadow) */
  --n-100: oklch(0.962 0.003 75); /* #f4f2f0 */
  --n-200: oklch(0.940 0.004 75); /* #edebe8 */
  --n-300: oklch(0.918 0.004 75); /* #e5e3e1 */
  --n-400: oklch(0.900 0.004 75); /* #dfdedb */
  --n-500: oklch(0.850 0.005 75); /* #d0cdca */
  --n-600: oklch(0.780 0.006 75); /* #bab7b3 */
  --n-700: oklch(0.585 0.006 75); /* #7e7b78 */
  --n-800: oklch(0.540 0.006 75); /* #716e6b */
  --n-900: oklch(0.480 0.006 75); /* #605d5a */
  --n-1000: oklch(0.215 0.006 75);/* #1b1916 */
  --accent: oklch(0.545 0.175 255);       /* #0a6ed2 white text 4.96:1 */
  --accent-hover: oklch(0.500 0.155 255); /* #1162b9 */
  --accent-text: oklch(0.505 0.160 255);  /* #0c63bd 5.83:1 */
  --focus: oklch(0.580 0.190 255);        /* #0278e7 4.27:1 (≥3:1 non-text) */
  --success: oklch(0.540 0.125 155);      /* #19834d */
  --warning: oklch(0.700 0.150 70);       /* #d98b09 fill/icon only */
  --warning-text: oklch(0.555 0.125 62);  /* #a65f08 text 4.83:1 */
  --danger:  oklch(0.560 0.200 25);       /* #d02b31 */
  --success-text: var(--success); --danger-text: var(--danger);
  --id-1: oklch(0.62 0.14 20);  --id-2: oklch(0.62 0.14 60);  --id-3: oklch(0.62 0.12 105);
  --id-4: oklch(0.62 0.14 150); --id-5: oklch(0.62 0.105 195); --id-6: oklch(0.62 0.12 240);
  --id-7: oklch(0.62 0.14 285); --id-8: oklch(0.62 0.14 330);
  --ring: 0 0 0 1px oklch(0 0 0 / 0.08);
  --hl: 0 0 #0000;
  --shadow-1: 0 1px 0 oklch(0 0 0 / 0.03), 0 1px 2px oklch(0 0 0 / 0.05);
  --shadow-pop: var(--ring), 0 4px 8px -2px oklch(0 0 0 / 0.08), 0 12px 24px -6px oklch(0 0 0 / 0.12);
  --shadow-modal: var(--ring), 0 8px 16px -4px oklch(0 0 0 / 0.10), 0 24px 48px -12px oklch(0 0 0 / 0.18);
  --scrim: oklch(0.2 0.01 75 / 0.25);
}
/* "System" theme: the token build emits the light block a second time as
   @media (prefers-color-scheme: light) { :root:not([data-theme]) { …same declarations… } }
   and Rust forwards Tauri WindowEvent::ThemeChanged (macOS/Windows; not Linux) [V-doc]. */

/* ---------- base ---------- */
html { font-family: var(--font-ui); font-size: var(--text-13); line-height: var(--lh-13);
  font-feature-settings: var(--font-features); font-optical-sizing: auto;
  background: var(--bg-0); color: var(--text-1); -webkit-font-smoothing: antialiased;
  overscroll-behavior: none; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
::selection { background: color-mix(in oklch, var(--accent) 35%, transparent); }
.num, time, [data-num] { font-variant-numeric: tabular-nums; }
@media (prefers-reduced-motion: reduce) {
  :root { --dur-1: 0ms; --dur-2: 0ms; --dur-3: 100ms; --dur-4: 100ms; --dur-spring: 100ms;
          --ease-spring: var(--ease-out); }   /* keep short opacity fades, kill movement */
}
```

### 5.2 The theme generator (Linear-style, 3 inputs)

`base = {h, c}`, `accent = {h}`, `contrast ∈ [0, 100]` (default 50).

- Surfaces and the `n-*` scale come from a fixed table of lightness values. `contrast` stretches the distance between the text and background lightness: at 100, dark `--text-2` goes to L 0.86 and `--border-1` to L 0.34.
- The accent L is solved so that `on-accent` is ≥ 4.5:1 and `focus` is ≥ 3:1 against `--bg-1`.
- After generation, **CI checks every pair** in §5.3 with WCAG 2.1 and APCA (the script used for this report is described below).
- Output is a CSS file per theme plus a JSON file for the canvas world palette. Runtime switching is just swapping `data-theme`.
- **No relative color syntax at runtime.** It needs Safari 18 [V-local, MDN BCD], which ties Codly to macOS 13+ users who have updated Safari [U].

The calculation script used for this report is at `/private/tmp/claude-501/-Users-julien-Documents-Repos-Codly/ae7e9f7f-d897-4679-8080-c22a339ba5b7/scratchpad/scripts/tokens.py`. It converts OKLCH to sRGB and computes WCAG and APCA-W3 0.0.98G. Port it to TypeScript (or use `culori` 4.0.2, MIT) inside the token build.

### 5.3 Contrast verification (computed today)

| Pair | Dark WCAG / APCA Lc | Light WCAG / APCA Lc | Target |
|---|---|---|---|
| `text-1` on `bg-1` | 16.31 / −96.4 | 17.17 / 102.9 | body ✔ |
| `text-2` on `bg-1` | 9.42 / −63.0 | 6.41 / 81.0 | secondary ≥ 4.5 / ≥ 60 ✔ |
| `text-2` on `bg-2` / `bg-0` | 8.95 / −62.5 | 6.03 / 76.8 | ✔ |
| `text-3` on `bg-1` | 5.61 / −40.1 | 4.11 / 67.6 | placeholder and meta; light is under 4.5, so **never use it for essential text** |
| `border-1` on `bg-1` | 1.38 | 1.32 | decorative hairline (by design) |
| `focus` on `bg-1` | 7.59 | 4.27 | non-text ≥ 3:1 ✔ |
| `on-accent` on `accent` | 4.76 / −77.1 | 4.96 / −78.3 | button label ≥ 4.5 ✔ |
| `on-accent` on `accent-hover` | 5.6 (darker hover) | – | ✔ |
| `accent-text` on `bg-1` | 9.45 | 5.83 | links ✔ |
| `danger` on `bg-1` | 6.52 | 5.05 | ✔ |
| `warning` / `warning-text` on `bg-1` | 11.02 | 2.70 (fill) / **4.83** (text) | use `--warning-text` for text in light |
| `id-*` on `bg-1` | about 7.7–8.3 | about 3.5–3.8 | identity colors are **fills** (shirts, rings, lanes), not text |

### 5.4 Browser-feature floor for WKWebView (Safari versions from MDN BCD, fetched today) [V-local]

| Feature | Safari | Used for |
|---|---|---|
| `oklch()`, `oklab()` | 15.4 | all tokens |
| `color-mix()` | 16.2 | `--accent-soft`, selection |
| relative color syntax `oklch(from …)` | 18 | **not used** |
| `light-dark()` | 17.5 | not used (data-theme instead) |
| `linear()` easing | 17.2 | `--ease-spring`, with an ease-out fallback |
| `image-rendering: pixelated` | 10 | world canvas |
| `OffscreenCanvas` and its 2D context | 16.4 | static world layer, future worker |
| `popover` attribute | 17 | optional for menus |
| `document.startViewTransition` | 18 | not used for core flows |

Tauri's default `minimumSystemVersion` is **10.13** [V-doc]. Set `bundle.macOS.minimumSystemVersion: "13.0"` and feature-detect `linear()`.

---

## 6. App layout and view sync

### 6.1 Window

- Tauri 2.12.5 (crate) and `@tauri-apps/api` 2.12.4 [V-local].
- `titleBarStyle: "Overlay"`, `hiddenTitle: true`, and `trafficLightPosition` require `decorations: true` [V-doc]. Mark the title-bar regions with `data-tauri-drag-region`. Known caveat: an unfocused window can't be dragged (tauri#4316) [V-doc].
- Set `backgroundColor` to the `--bg-0` value to avoid a white flash.
- Keep `backgroundThrottling` at its default, which suspends a hidden view after about 5 minutes [V-doc]. Consequences:
  - The Rust core owns all state and all notifications.
  - On `visibilitychange → visible`, the UI requests a snapshot plus the event cursor and fast-forwards the world. It does not replay missed animations.
- Translucent sidebar (`window-vibrancy` 0.8.1): **off by default**. A solid surface keeps token contrast predictable. It can be offered as a setting.

### 6.2 Workspace view (1440×900)

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ ●●●   acme/web ▾   main ▾        [⌘K  Search, run, or ask…            ]   ◉3 running  ⚑1  ▮▮▯ 62% 5h │ 40 title (drag)
├──────────────┬───────────────────────────────────────────────────────────────┬────────────────────────┤
│ ⚑ Inbox    1 │ [Workspace|World|Split]   Chat  Timeline  Board  Graph  Diffs  │ ◧ Ada        ⋯  ✕      │ 36 tabs
│ ◎ My tasks   │───────────────────────────────────────────────────────────────│ Claude Code · sonnet   │
│              │ TIMELINE                                     ◷ live  ⏸  ⇤ ⇥   │ ● Editing  · 4m12s     │
│ AGENTS     + │  Ada  ▇▇▇░▇▇▇▇▇▇▇▇▇▇▇▇▇░▇▇▇  Edit src/app.ts               │────────────────────────│
│ ● Ada  edit  │  Bo   ▇▇░░░░░░░░░░░░░░░░⚑    needs approval: rm -rf dist   │ NOW                    │
│ ⚑ Bo   wait  │  Cy   ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇  npm test · 00:41             │ Editing src/app.ts     │
│ ● Cy   test  │      14:02     14:03     14:04     14:05                    │ Why? ▸                 │
│ ◌ Dee  idle  │───────────────────────────────────────────────────────────────│ TASK  #14 Auth guard   │
│              │ CHAT                                                          │ ⛓ blocked by #12       │
│ VIEWS        │  You    Add an auth guard to /settings and tests.             │ PLAN  ■■■□□ 3/5        │
│ ▤ Board      │  Codly  Split into 3 tasks → Ada #14, Bo #15, Cy #16   Why? ▸ │ FILES  read 6 · mod 2  │
│ ⋔ Graph      │  Ada    Edited 2 files (+48 −6)                    View diff  │ COST ≈ $0.42 list · 38k│
│ ◷ Timeline   │ ┌─────────────────────────────────────────────────────────┐   │ ▸ Reasoning summary    │
│              │ │ Message Codly…  @agent #task /file          ⌘↵ Send     │   │ ▸ Live terminal        │
│ ⚙ Settings   │ └─────────────────────────────────────────────────────────┘   │ ▸ Activity log         │
└──────────────┴───────────────────────────────────────────────────────────────┴────────────────────────┘
  232 sidebar (⌘[ / [)            flexible                                      384 inspector (⌘\)
```

### 6.3 World view

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ ●●●   acme/web ▾          [⌘K …]                                       ◉3 running  ⚑1  ▮▮▯ 62% 5h   │
├──────────────┬───────────────────────────────────────────────────────────────┬────────────────────────┤
│ (sidebar)    │ ┌ World ─────────────────────────────── 3× ─ + 0 ◎follow ▾ ┐  │ (same Inspector)       │
│              │ │  LIBRARY │ MEETING     ┌───────────┐                       │  │                        │
│              │ │  ▥▥ ▥ ▦  │  [WB]       │ "Reading  │                       │  │                        │
│              │ │          D             │  app.ts…" │                       │  │                        │
│              │ │ ###D#####│####D#####   └────┬──────┘                       │  │                        │
│              │ │   [▭][▭]   [▭][▭]   [▭][▭]   ☺ Ada  (selected: outline)    │  │                        │
│              │ │   ☺  ☺      ☺  ⚑Bo                                        │  │                        │
│              │ │ ###D#######D######D###                                     │  │                        │
│              │ │  SERVER ▦▦ ☺Cy »   KITCHEN ☕   MAIL ✉        D (entrance) │  │                        │
│              │ └──────────────────────────────────────────────────────────────┘ │                        │
│              │ ┌ agent strip ──────────────────────────────────────────────┐  │                        │
│              │ │ [☺ Ada ● edit 4m] [☺ Bo ⚑ approve] [☺ Cy ● test 0:41] [+]│  │                        │
│              │ └───────────────────────────────────────────────────────────┘  │                        │
└──────────────┴───────────────────────────────────────────────────────────────┴────────────────────────┘
 HUD: zoom − / + / fit (0), follow toggle (F), freeze view (Space — pauses rendering only, never agents),
 ticker of last 3 events (bottom-left, aria-live mirrors it), mini-legend on hover over stations.
```

### 6.4 Split view (default at ≥ 1440 px wide; stacked below that)

```
┌──────────────┬──────────────────────────────┬──────────────────────────────┬──────────────────┐
│ sidebar      │ WORLD (42%)                  │ WORKSPACE (58%)              │ inspector        │
│              │  click agent ───────────────────► selection store ◄────────── click row       │
│              │  hover agent ── outline ◄──────── hover store ──────► row highlight           │
│              │  camera follows selection     │ timeline scrolls to agent    │ opens for agent  │
└──────────────┴──────────────────────────────┴──────────────────────────────┴──────────────────┘
 ⌘1 Workspace · ⌘2 World · ⌘3 Split · drag divider (snaps 33/42/50%) · double-click divider = reset
```

### 6.5 Sync model

The **event log is the single source of truth.** It is persisted in SQLite by Rust and streamed over a Tauri `Channel`. Every view is a projection of it:

```
Rust adapters ─► AgentEvent (raw, persisted) ─► WorldActivityEvent (normalized, persisted)
                                    │                         │
                       Timeline/Board/Graph/Chat        World reducer (pure TS)
                                    └──────► UI store ◄───────┘
UI store (tiny, framework-agnostic signals):
  selection: { agentId?, taskId?, fileId?, eventSeq? }   // one selection, shared
  hover:     { agentId?, taskId? }                       // transient, 0-ms propagation
  timeCursor: 'live' | seq                               // scrubbing sets world + timeline
  view: 'workspace' | 'world' | 'split'; inspectorOpen: boolean
```

- Selecting anywhere sets `selection`.
  - The world draws an outline, and the camera follows if the agent is off-screen (it never snaps if the agent is visible).
  - The timeline scrolls the lane into view.
  - The inspector opens.
- **The selection is part of the URL-like route** (`codly://project/acme/agent/ada?seq=812`), so it deep-links and back/forward work (Vercel guideline [V-doc]).
- Scrubbing the timeline sets `timeCursor`. The world re-derives its state at that `seq` by replaying from the nearest snapshot, using seeded RNG and the event timestamps as the clock. A "LIVE" pill returns to the present.

---

## 7. Component inventory

| Component | Purpose | Key states | Keyboard / notes |
|---|---|---|---|
| `AppShell` | title bar, sidebar, main, inspector grid | sidebar collapsed, inspector open | `[` / `⌘[` toggles sidebar, `⌘\` toggles inspector |
| `TitleBar` | project and branch switchers, palette trigger, global status | offline, rate-limited | drag region |
| `CommandPalette` | universal search, commands and ask | scopes, loading (≥ 300 ms delay), empty | `⌘K`, `↵`, `⌘↵`, `⌘K` for the action panel, `Esc` |
| `Sidebar` / `NavItem` | inbox, tasks, agents, views | active, badge, dimmed (Linear 2026) | `G I`, `G T`, `G B`, `G G` (graph), `G W` (world) |
| `AgentRow` / `AgentChip` | name, pixel portrait (16×16 head crop at 2×), status | running, approval, blocked, idle, offline | `J`/`K`, `↵` |
| `StatusDot` + label | redundant status cue | uses icon shape **and** color | – |
| `Tabs` (compact pills) | Chat, Timeline, Board, Graph, Diffs, Code, Terminal | overflow menu | `⌃Tab` |
| `SplitPane` | World ↔ Workspace | snap points | double-click resets |
| `Inspector` | explainable agent and task panel (§8) | agent / task / file subject | `⌘\`, `⌥↑`/`⌥↓` between sections |
| `WhyExpander` | progressive-disclosure provenance | collapsed, expanded | `⌘.` on a focused row toggles "Why?" |
| `ProvenanceBadge` | Observed, Inferred, Provided, Model-stated | – | tooltip explains each |
| `CitationChip` | retrieval citation (file:lines, score, match type) | hover preview | `Space` peeks |
| `FileChip` | path, diffstat, reason | read, modified, created, deleted | `⌘O` opens, `⌘⌃C` copies path |
| `TokenGauge` / `CostLine` | context snapshot, list-price cost | near-limit (≥ 80%) | numbers tabular |
| `RateLimitMeter` | 5-hour and 7-day utilization (Claude `rate_limit_event`) [V-local fixture] | allowed, near, limited | – |
| `ApprovalCard` | exact command or diff, risk hints, actions | pending, approved, denied, expired | `⌘↵` once, `⌥⌘↵` always (this session), `⌘⌫` deny, `⇧⌘⌫` deny with note |
| `Inbox` | queue of approvals and questions | empty state | `A` approve, `⇧A` always, `D` deny, `E` edit (if the adapter supports `updatedInput`) |
| `TimelineLane` | per-agent activity bars (identity color) | live, scrubbed | `←`/`→` steps through events |
| `KanbanBoard` / `TaskCard` | backlog → running → review → done | blocked overlay | `C` create, `X` select, `⌘⌫` archive |
| `DependencyGraph` | task DAG (dagre layout: `@dagrejs/dagre` 3.1.3, MIT [V-local]) | critical path highlighted | arrow keys move between nodes |
| `DiffViewer` | unified or split, per-hunk approve | stale | `]`/`[` next and previous hunk |
| `CodeViewer` | read-only, highlights "lines the agent read" | – | `⌘F` |
| `TerminalPane` | xterm (`@xterm/xterm` 6.0.0 plus webgl addon 0.19.0 [V-local]) | mirror (read-only) or "take control" | `cursorBlink` only when focused (CPU) |
| `Toast` | progress, success, info, error | stacked ≤ 3 | polite live region |
| `EmptyState` | one line plus an action | – | – |
| `Kbd`, `Tooltip`, `Menu`, `ContextMenu`, `Dialog`, `Popover` | primitives | – | `@floating-ui/dom` 1.8.0 for positioning [V-local] |
| `Composer` | prompt input with `@`, `#`, `/` mentions | streaming, disabled | `⌘↵` sends (textarea rule [V-doc]) |
| **World:** `WorldCanvas`, `WorldOverlay` (DOM bubbles and tags), `WorldHUD`, `AgentStrip`, `EventTicker`, `ReplayScrubber` | §9 | live, scrubbed, frozen, reduced-motion | §9.10 |
| `PixelPortrait` | the same procedural recipe, head-and-shoulders crop | – | used in every list, so identity is consistent across views |

**Icons.** `lucide` 1.61.0 (ISC) [V-local] at 16 px, stroke 1.5, using `--icon`. Status glyphs (●, ⚑, ◌, ✕) are drawn as inline SVG. World icons are pixel art.

---

## 8. Explainable orchestration UX: the Inspector

### 8.1 Section order and default state

| # | Section | Default | Content |
|---|---|---|---|
| 1 | Header | always | portrait, name, provider · model, status pill, elapsed, worktree branch, `⋯` menu (Pause, Stop (⇧⌘. with confirm), Open terminal, Re-roll look) |
| 2 | **Now** | open | verb + target + since ("Running `npm test` · 41 s"), **Why? ▸** (§8.2), the plan item it belongs to |
| 3 | **Needs you** | open when present | `ApprovalCard` or question; nothing else competes visually |
| 4 | Task | open | title, acceptance criteria, assigned by (user or orchestrator, with the "Why this agent?" rationale), dependencies (`blocked by`, `blocks`) |
| 5 | Plan | collapsed when >5 items | live checklist from TodoWrite, plan or todo_list |
| 6 | Files | summary line | "read 6 · modified 2 · created 1". Expanding shows each file with a reason badge and a diffstat |
| 7 | Reasoning summary | collapsed | model-provided summaries, each labelled with its source; "Show raw" |
| 8 | Cost & context | one line | tokens in/out/cache, context gauge (snapshot), list price with the billing label, rate-limit window |
| 9 | Live terminal | collapsed | xterm mirror; "Take control" asks first |
| 10 | Activity log | collapsed | normalized events with filters (tool, file, error); each row links to the raw event JSON |

**Progressive disclosure rule.** The first screen (no scrolling at 900 px tall) answers three questions: *what is it doing, does it need me, what has it touched*. Everything else is one click away. Expanded and collapsed state is remembered per section, not per agent.

### 8.2 The "Why?" model

Each fact carries one provenance badge.

| Badge | Meaning | Example | How it's computed |
|---|---|---|---|
| **Observed** | it came verbatim from a CLI event | "Read `src/app.ts`" | the tool_use, item or tool_call event (id shown) |
| **Inferred** | a Codly heuristic, which can be wrong | "Found via `rg useAuth` at 14:02" | a path that appears in an earlier search tool's output and is then read → link the two; "tests: 128 passed" parsed from output |
| **Provided** | Codly put it in the agent's context | "Included by Codly: matched symbol `useAuth` (BM25 12.3), imported by `routes.ts`" | retrieval citations from the Rust index: query, match type (symbol, path, text, embedding), score, line ranges, rank |
| **Model-stated** | the model's own words | "Intent: read README then reply" | thinking or reasoning summaries, `update_topic.strategic_intent`, `post_turn_summary` |

The "Why this agent?" rationale for the orchestrator's assignment is shown as Provided plus Model-stated. Example: "Chosen: Claude Code (Sonnet). Reason: TypeScript refactor; your rule 'TS → claude'. Alternatives: Codex (busy), Gemini (rate-limited until 14:00)."

**Educational tone.** Every badge and jargon term (worktree, context window, cache read, tool call) has a one-line tooltip in plain English. An "Explain more" link in the `⋯` menu turns on inline glossary text for new users.

### 8.3 Where the reasoning summary comes from

| Source | Field | Status |
|---|---|---|
| Claude Code stream-json | `thinking` content blocks (when present), `system/post_turn_summary` (`status_category`, `status_detail`, `needs_action`), `tool_use_summary.summary`, `system/task_progress.summary` | `post_turn_summary` observed in today's fixture (CLI 2.1.309) [V-local]. `tool_use_summary` and `task_progress` are in Agent SDK 0.3.310 types [V-local d.ts]; emission in the CLI not verified |
| Codex `exec --json` | `item.*` of type `reasoning` (`text`) and `agent_message` | [V-src codex-rs/exec/src/exec_events.rs @ 970111f] |
| Gemini CLI `--output-format stream-json` | `tool_use` with `tool_name: "update_topic"` → `parameters.strategic_intent` | observed in fixture (Gemini CLI 0.62.0) [V-local] |
| ACP agents (Gemini `--acp`, OpenCode) | `session/update` → `agent_thought_chunk` | observed for Gemini [V-local fixture] |
| Codly orchestrator / Deep Agents backend | its own plan, `write_todos`, routing rationale | Codly-authored → labelled Provided or Model-stated |

### 8.4 Cost and context display

- **Claude.** `result.total_cost_usd`, plus `modelUsage[model].{inputTokens, outputTokens, cacheReadInputTokens, cacheCreationInputTokens, contextWindow, costUSD, costBasis}`. Today's fixture had `costBasis: "list"` and `contextWindow: 1000000` [V-local]. Render it as **"≈ $0.0037 at list price · included in your plan"** when on a subscription.
- **Codex.** `turn.completed.usage {input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens}` [V-src]. There is no cost field, so show tokens only. An estimate is allowed if labelled "est.".
- **Gemini.** `result.stats {total_tokens, input_tokens, output_tokens, cached, duration_ms, tool_calls, models{…}}` [V-local].
- **Context gauge.** `input + cache_creation + cache_read + output` of the **latest** turn, divided by the provider-declared window. Never sum turns (the Pixel Agents lesson [V-src]).
- **Rate limit.** Claude `rate_limit_event.rate_limit_info.unifiedWindows.{five_hour, seven_day}.utilization` and `resetsAt` [V-local]. This drives the title-bar meter and the world's "sleep until 14:00".

### 8.5 Approvals (exact semantics)

- **ACP.** `session/request_permission` with `options[{optionId, name, kind}]`, where `kind ∈ allow_once | allow_always | reject_once | reject_always`. The response is `{"outcome":{"outcome":"selected","optionId":…}}`, or `cancelled` if the turn is cancelled [V-doc agentclientprotocol.com].
- **Claude Code.** Approval arrives via `PermissionRequest` hooks or a `can_use_tool` control request (SDK control subtype present [V-local d.ts]). `system/permission_denied` reports denials. `session_state_changed.state === 'requires_action'` is the session-level signal [V-local d.ts]. Its doc comment says `idle` is the "authoritative turn-over signal".
- **Codex.** `command_execution.status` can be `declined` [V-src]. The interactive approval flow belongs to the app-server protocol, which is out of scope here.

**`ApprovalCard` layout.**
```
┌ ⚑ Bo wants to run a command ─────────────────────────── 14:05 · 12 s ago ┐
│  $ rm -rf dist && npm run build                              [Copy ⌘⇧C]  │
│  cwd  ~/.codly/worktrees/acme-web/bo-15   (inside Bo's worktree ✓)       │
│  Why? ▸  Task #15 "Fix build" · plan step 2/4 · Model-stated: "clean …"  │
│  Risk  deletes files · no network · stays in worktree                    │
│  [ Approve once ⌘↵ ]  [ Always for this session ⌥⌘↵ ]  [ Deny ⌘⌫ ] [Deny + note ⇧⌘⌫] │
└───────────────────────────────────────────────────────────────────────────┘
```
Risk hints are **Inferred** (static checks: path outside the worktree, `rm`, `curl | sh`, `git push --force`, network). They are labelled as such.

### 8.6 Bubble copy: derived from real events

Rules:
- Present progressive verb plus a short target, ≤ 28 characters, `…` suffix while running.
- Show file basenames only; the full path goes in the tooltip.
- Commands show the first 2 tokens, monospace.
- **No invented feelings.** Completion bubbles state the outcome.

| Event (normalized) | Running bubble | End bubble (2 s, then fade) |
|---|---|---|
| read `src/app.ts` | "Reading app.ts…" | – (silent) |
| read × N within 3 s | "Reading 12 files…" | – |
| search `useAuth` | "Searching “useAuth”…" | "Found 7 matches" (if count known; Inferred) |
| glob `**/*.tsx` | "Looking for *.tsx…" | – |
| edit / create / delete | "Editing app.ts…" / "Creating auth.ts…" / "Deleting old.ts…" | "+48 −6" |
| execute `npm run build` | "Running npm run…" plus an elapsed chip | "✓ exit 0" / "✕ exit 1" |
| test | "Running tests…" | "Tests pass ✓ 128" / "3 tests failing" |
| fetch `docs.rs` | "Reading docs.rs…" | – |
| mcp `linear.create_issue` | "Asking linear…" | – |
| plan update | "Updating the plan…" | "Plan 3/7" |
| delegate | "Briefing a helper…" | – |
| message → Bo | (paper plane) "To Bo: {first 20 chars}…" | – |
| await_approval | "May I run rm -rf…?" (persistent, amber) | – |
| await_input | "Your turn ↵" (persistent) | – |
| blocked | "Stuck: {error class}" (persistent) | – |
| rate_limited | "Out of juice until 14:00" | – |
| compacting | "Tidying notes…" | "180k → 40k tokens" |
| done | – | "Done ✓" |

**Sanitization.** Bubble and tag text derives from untrusted content: file names, commands, model output. Render it with `textContent` only. Strip C0/C1 controls, bidi overrides (U+202A–202E, U+2066–2069) and zero-width characters. Collapse whitespace. Truncate by grapheme (`Intl.Segmenter`). Never linkify inside the world. The Claude SDK applies similar normalization to peer names (its `SDKMessageOrigin.name` doc comment lists stripped categories Cc/Cf/Cs/Zl/Zp) [V-local d.ts], which confirms this is a real concern.

---

## 9. The world engine

### 9.1 Architecture

```
Tauri Channel<WorldActivityEvent>  (ordered, from Rust normalizer; replayable by seq)
        │
        ▼
 worldReducer(state, event) ── pure, deterministic (seeded RNG, event ts as clock)
        │   produces: per-agent ActivityState + intents (goTo station, playPose, spawnFx)
        ▼
 Simulation.step(dt)  ── locomotion (A*), pose timers, fx particles, camera
        │   returns { moving: boolean, nextStepAt: number }
        ▼
 Renderer (Canvas 2D) ── static layer blit + z-sorted drawables + DOM overlay positions
        ▲
 FrameScheduler ── rAF only while moving; step timers for loops; nothing when static/hidden
```

The engine is a framework-free TS module: no DOM except the canvas and overlay root. That keeps it testable in Node and lets it move into a Worker with `OffscreenCanvas` later (Safari 16.4 [V-local BCD]). **v1 runs on the main thread.** The workload is small (≤ 64×64 tiles, ≤ 24 agents) and it avoids message-passing for hit tests.

### 9.2 Normalized activity schema (emitted by Rust)

```ts
export type ActivityKind =
  | 'spawn' | 'despawn' | 'idle' | 'thinking' | 'replying'
  | 'read' | 'search' | 'edit' | 'create' | 'delete' | 'move'
  | 'execute' | 'test' | 'fetch' | 'mcp'
  | 'plan' | 'delegate' | 'message' | 'review' | 'commit' | 'merge'
  | 'await_approval' | 'await_input' | 'blocked' | 'retrying' | 'rate_limited' | 'compacting' | 'done';

export interface WorldActivityEvent {
  seq: number;                 // monotonic per project (SQLite rowid)
  ts: number;                  // source timestamp, ms epoch
  agentId: string;             // Codly agent (stable across sessions)
  activityId: string;          // tool_use id / item id / toolCallId — pairs start/update/end
  phase: 'start' | 'update' | 'end';
  kind: ActivityKind;
  target?: { path?: string; paths?: string[]; command?: string; query?: string; url?: string;
             peerAgentId?: string; server?: string; tool?: string; text?: string };
  outcome?: 'ok' | 'error' | 'denied' | 'cancelled';
  meta?: { exitCode?: number; testsPassed?: number; testsFailed?: number; elapsedMs?: number;
           added?: number; removed?: number; planDone?: number; planTotal?: number;
           resetsAt?: number; preTokens?: number; postTokens?: number; attempt?: number };
  provenance: { provider: 'claude' | 'codex' | 'gemini' | 'opencode' | 'cursor' | 'copilot' | 'pi' | 'deepagents';
                rawType: string; rawId?: string; confidence: 'observed' | 'inferred' };
}
```

**Classification rules for unknown or new tools.** Tool sets differ by CLI version and config. Today's Claude 2.1.309 init listed `Task, Bash, Edit, Read, Write, WebFetch, WebSearch, NotebookEdit, ToolSearch, SendMessage, ListAgents, Monitor, LSP, Skill, EnterWorktree, …`, but **not** `Grep`, `Glob` or `TodoWrite` [V-local fixture]. So:
1. Use the provider's tool-kind table (as Pixel Agents does via `providerCapabilities` [V-src]).
2. Use ACP `kind` directly when present.
3. Fall back to the input shape: `file_path`/`path` plus write content → edit; `file_path` only → read; `command` → execute; `pattern`/`query`/`glob` → search; `url` → fetch; `mcp__<server>__<tool>` → mcp.
4. Classify shell commands by argv[0]: `cat/sed -n/head/tail/bat/less` → read (Inferred); `rg/grep/fd/find/ls/tree` → search (Inferred); `git commit` → commit; `git merge`/`gh pr merge` → merge; the test-runner regex → test.

**Test detection** (Inferred, in Rust):
`(^|[\s;&|])((p?npm|yarn|bun)( run)? test|vitest|jest|pytest|cargo (test|nextest)|go test|swift test|xcodebuild .*test|rspec|mix test|phpunit|deno test|playwright test)\b`.
Parse counts from the common summaries:
- vitest: `Tests  12 passed`
- jest: `Tests: 3 failed, 10 passed`
- pytest: `== 10 passed, 2 failed in`
- cargo: `test result: ok. 12 passed; 0 failed`

### 9.3 Raw → normalized mapping per CLI (earliest signal first)

| Normalized | Claude Code `stream-json` [V-local fixture 2.1.309 / V-local SDK d.ts] | Codex `exec --json` [V-src] | Gemini `stream-json` [V-local 0.62.0] | ACP (Gemini `--acp`, OpenCode 1.18.31) [V-doc / V-local] |
|---|---|---|---|---|
| spawn | `system/init` | `thread.started` | `init` | `session/new` result |
| thinking | `system/status{status:"requesting"}`, `stream_event content_block_start{type:"thinking"}`, `system/thinking_tokens` | `item.started{type:"reasoning"}` | (none observed) | `agent_thought_chunk` |
| tool start (pre-pose) | `stream_event content_block_start{content_block:{type:"tool_use",name}}` (name known, input empty) | `item.started` (command_execution, file_change, mcp_tool_call, web_search, collab_tool_call) | `tool_use{tool_name, tool_id, parameters}` | `tool_call{kind,title,status:"pending"}` |
| tool target known | `assistant.message.content[].tool_use{name,input}` | same item (`command`, `changes[].path`) | same | `tool_call_update{status:"in_progress", locations[].path, rawInput}` |
| tool progress | `tool_progress{tool_name, elapsed_time_seconds}` (SDK type) | `item.updated` | – | `tool_call_update` |
| tool end | `user.message.content[].tool_result{tool_use_id,is_error?}` | `item.completed{status, exit_code}` | `tool_result{tool_id,status}` | `tool_call_update{status:"completed"/"failed"}` |
| replying | `stream_event content_block_delta` (text) | `item.*{type:"agent_message"}` | `message{role:"assistant",delta:true}` | `agent_message_chunk` |
| plan | TodoWrite tool (when present) | `todo_list` item | `write_todos` [U] | `plan{entries[{content,priority,status}]}` |
| delegate | `Task`/`Agent` tool + `system/task_started{task_id, description, subagent_type}`; child events carry `parent_tool_use_id` | `collab_tool_call{tool:"spawn_agent"}` | – | – |
| message (agent↔agent) | `SendMessage` tool; peer origin on user messages | `collab_tool_call{tool:"send_input"}` | – | Codly orchestrator events |
| await_approval | `PermissionRequest` hook / `can_use_tool` / `session_state_changed{state:"requires_action"}` | `command_execution.status:"declined"` (after the fact) | – | `session/request_permission` |
| retrying | `system/api_retry{attempt,max_retries,retry_delay_ms}` | – | – | – |
| rate_limited | `rate_limit_event{rate_limit_info.status ≠ "allowed"}` | `error` / `turn.failed` (429) [U] | `result{status:"error"}` [U] | prompt error [U] |
| compacting | `system/status{status:"compacting"}`, `system/compact_boundary{compact_metadata:{pre_tokens,post_tokens}}` | – | – | – |
| blocked | `result{subtype:"error_*"}`, `system/permission_denied` | `turn.failed{error}`, `error` item | `result{status:"error"}` | JSON-RPC error |
| done / idle | `result{subtype:"success"}` → `session_state_changed{state:"idle"}` ("authoritative turn-over") | `turn.completed` | `result{status:"success"}` | `session/prompt` result `stopReason` |

Source code sanity checks:
- Codex `ThreadItemDetails` is `agent_message | reasoning | command_execution | file_change | mcp_tool_call | collab_tool_call | web_search | todo_list | error`.
- `CommandExecutionStatus` is `in_progress | completed | failed | declined`.
- `PatchChangeKind` is `add | delete | update`.
- `CollabTool` is `spawn_agent | send_input | wait | close_agent` [V-src].
- ACP `ToolKind` is `read | edit | delete | move | search | execute | think | fetch | switch_mode | other`, and `ToolCallStatus` is `pending | in_progress | completed | failed` [V-doc].

### 9.4 Two-layer state machine

**Activity layer** (per agent; set instantly by the reducer). When several activities are live, the highest priority wins the pose; the rest appear as stacked mini-icons ("×3").

```
priority:  await_approval > blocked > await_input > rate_limited > retrying > compacting
         > execute/test(long) > edit/create/delete > search > read > fetch/mcp > message/delegate
         > plan/review/commit/merge > replying > thinking > done > idle
```

**Locomotion/pose layer** (time-based):

```
            spawn                       activity start (excursion-eligible & still live @400ms)
 [offline] ─────► [walking:door→desk] ─► [at_desk] ─────────────────────────► [walking:→station]
                                          ▲   │ pose = f(activity)                   │ arrive
                                          │   │                                       ▼
                       activity end /      │   └── in-place poses: type/read/think   [at_station]
                       higher priority     │       raise_hand/slump/sleep             │ pose = f(activity)
                       elsewhere           │                                          │ activity end
                                          └──────────── [walking:→desk] ◄─────────────┘
 despawn: any → [walking:→door] → fade (0.3 s) → [offline]
 reduced motion: every walking:* becomes a 150 ms crossfade-teleport.
```

**Timing rules** (the honesty contract):
1. **≤ 250 ms to truth.** The overlay icon and bubble update on the event, even while the character is mid-walk.
2. **Minimum dwell of 600 ms per pose.** Rapid events coalesce: the pose holds, and the icon and bubble show the latest event with a count.
3. **Excursion threshold.** Walk to a station only if the activity is still live after 400 ms and its kind is excursion-eligible (execute, test, search > 1.2 s, plan, review, commit, merge, fetch). If it ends mid-walk, turn around immediately.
4. **No phantom work.** If the agent is idle, nothing moves unless Ambient life is on, and even then the bubble and tooltip say "Idle since 14:02".
5. **Long idle is time-derived from the last real event.** After 10 minutes the agent shows `sleep` with "zZ" (static, no loop).

### 9.5 Event → animation mapping (full table)

| Activity | Where | Pose and frames | Overlay / FX | Bubble | Ends when |
|---|---|---|---|---|---|
| **spawn** | door → own desk | walk (4f @ 8 fps) then sit | door opens (2f) | "Hi, I'm Ada" (1.5 s) | arrival |
| **idle** | desk | `sit` (static) | none (0 CPU) | – | any activity |
| idle > 10 min | desk | `sleep` | "zZ" static sprite | – | any activity |
| **thinking** | desk | `sit_think` | thought cloud "…" (3-dot step at 2 Hz) | – (hover shows the summary's first line) | next activity |
| **replying** | desk | `sit_type` | monitor shows prose lines | "Writing a reply…" | message end |
| **read** | desk | `sit_read` (2f @ 600 ms) | paper prop; "×N" count chip | "Reading app.ts…" | tool end |
| **search** | desk; filing cabinet / bookshelf if > 1.2 s | `sit_read`, or at the station `stand` + 2f "flip" | magnifier icon | "Searching “useAuth”…" | tool end |
| **edit / create / delete / move** | desk | `sit_type` (2f @ 300 ms) | monitor code-scroll (2f); create = sparkle on monitor; delete = paper into bin (3f) | "Editing app.ts…" | tool end, then "+48 −6" for 2 s |
| **execute** (short) | desk | `sit_type` | `>_` icon | "Running git status…" | end |
| **execute** (long) | **server rack / terminal kiosk** | `stand` facing the rack | rack LEDs blink (2f @ 4 Hz); elapsed chip | "Running npm run…" + 0:41 | end → walk back |
| **test** | server rack | `stand` | LEDs | "Running tests…" | pass: `celebrate` (3f × 2) + confetti (12 one-pixel particles, 600 ms) + "Tests pass ✓ 128"; fail: `slump` 1 s + small rain puff + "3 tests failing" |
| **fetch** (web) | window / globe station | `stand` facing the window | globe icon | "Reading docs.rs…" | end |
| **mcp** | desk | `phone` (2f @ 500 ms) | handset prop | "Asking linear…" | end |
| **plan** | whiteboard (if items changed), else desk | `stand` + 2f "write" | sticky notes appear on the board = `planTotal`, checked = `planDone` | "Plan 3/7" | end |
| **review** | meeting table / whiteboard | `sit_read` at the table | diff sheet prop | "Reviewing 4 files…" | end |
| **delegate** | desk | `point` (1 s) | helper ("intern": 14×20 sprite, parent palette, badge) spawns next to the desk with a door-less pop (3f) | "Briefing a helper…" | helper despawns when the task ends |
| **message** (A → B) | desks | sender `point` 400 ms | **paper plane** arc from A to B (Munder envelope parameters: lift 38 px, 230 px/s, 0.8–2.0 s, 0.34 s arrival burst [V-src]); the receiver's desk gets an envelope icon until read | "To Bo: …" | plane lands |
| message (multi-turn, both idle-ish) | meeting table | both walk and sit | speech dots alternate | – | conversation ends (no event for 20 s) |
| **commit** | mailbox | `carry` (envelope) → drop (3f) | mailbox flag up (2f) | "Committing 3 files…" | end |
| **merge** | mailbox | `celebrate` (1 loop) | flag up + small confetti | "Merged into main ✓" | end |
| **await_approval** | desk (stands up) | `raise_hand` (2f @ 500 ms) | **amber "!" bubble, persistent**; desk lamp amber; agent card pulses (≤ 3 pulses, then static); dock badge | "May I run rm -rf…?" | resolved (approve → `sit`, deny → `slump` 600 ms) |
| **await_input** | desk | `raise_hand` (lower arm) | "?" bubble, persistent | "Your turn ↵" | user replies |
| **blocked** (turn failed / fatal error) | desk | `slump` | **rain cloud** (2f drizzle @ 4 Hz, stops after 8 s and freezes as a static cloud) | "Stuck: auth error" | next activity |
| **retrying** | desk | `sit` | "↻" spinner icon (4f) | "Retrying (2/10)…" | next success or failure |
| **rate_limited** | desk | `sleep` | battery-empty icon + countdown chip (updated once per minute, not every frame) | "Out of juice until 14:00" | `resetsAt` passes or the next event |
| **compacting** | desk | `sit_read` + paper shuffle (3f) | – | "Tidying notes…" | then "180k → 40k" |
| **done** | desk | `sit` | green check bubble, auto-fades after 2 s (Pixel Agents' value [V-src]) | "Done ✓" | 2 s |
| **despawn** | desk → door | walk | door opens; fade 0.3 s | – | gone |

### 9.6 Office layout (default 30×18 tiles, 16 px tiles = 480×288 art px)

```
     0    5    10   15   20   25  29
  0  ##############################
  1  #LIBRARY   #  MEETING/REVIEW #
  2  # BB BB FC #  WW    ┌──────┐ #      B bookshelf   FC filing cabinet   WW whiteboard (2×2)
  3  #          #        │table │ #      table = review seats ×4
  4  #     ·    D        └──────┘ #      D door (walkable)   · walkable floor
  5  ####D###########D#############
  6  #                            #      [d] desk pod seat = desk(2×1)+chair+monitor
  7  #  [d][d]    [d][d]    [d][d]#      pods of 4: 2 facing 2 (6 pods = 24 seats max)
  8  #  [d][d]    [d][d]    [d][d]#      desks assigned per agent (stable: persisted;
  9  #                            #        first free seat in the project's area)
 10  #  [d][d]    [d][d]    [d][d]#
 11  #  [d][d]    [d][d]    [d][d]#
 12  #                            #
 13  ####D########D#########D######
 14  # SERVER    #  KITCHEN #MAIL #      RR server racks (2 use-slots)  TK terminal kiosk
 15  # RR RR TK  #  CM  ☐   # MB  #      CM coffee (Ambient only)   MB mailbox
 16  #       ·   #     ·    #     D ◄── entrance: spawn/despawn
 17  ##############################
```

**Rules.**
- Each station has `useSlots: [{col,row,facing}]`. If every slot is busy, the agent performs the activity in place at its desk (fallback). It never queues out of sight.
- Areas map to projects or worktrees, like Pixel Agents "Areas" [V-src]. A second project gets a second floor, switched with `⌘⇧[` / `⌘⇧]`.
- The layout is JSON `{version, cols, rows, tiles[], furniture[], areas[]}`, compatible in spirit with Pixel Agents' layout model [V-src]. A layout editor is v2.
- Desk nameplate: agent name plus provider text, drawn in Geist Pixel or a 5×7 code font.

### 9.7 Pathfinding: A* (4-connected, turn penalty)

Both references use BFS [V-src]. Codly uses A* because:
- Manhattan plus a turn penalty gives straighter, more human paths.
- A binary heap scales to 64×64.
- Per-character passability is a predicate (an agent's own chair is passable).

Characters may overlap. Optional "polite yield": if the next tile is occupied by a walking character, wait 150 ms once.

```ts
const TURN = 0.15; const D = [[1,0],[-1,0],[0,1],[0,-1]] as const;
export function aStar(w: number, h: number, sx: number, sy: number, gx: number, gy: number,
                      passable: (x: number, y: number) => boolean): Array<[number, number]> | null {
  const N = w * h, id = (x: number, y: number) => y * w + x;
  const g = new Float32Array(N).fill(Infinity), came = new Int32Array(N).fill(-1);
  const dir = new Int8Array(N).fill(-1), heap = new MinHeap();            // ~25-line binary heap
  const hz = (x: number, y: number) => (Math.abs(x - gx) + Math.abs(y - gy)) * 1.001; // tie-break
  g[id(sx, sy)] = 0; heap.push(id(sx, sy), hz(sx, sy));
  while (heap.size) {
    const c = heap.pop(), cx = c % w, cy = (c / w) | 0;
    if (cx === gx && cy === gy) { const p: Array<[number, number]> = [];
      for (let n = c; n !== id(sx, sy); n = came[n]) p.push([n % w, (n / w) | 0]); return p.reverse(); }
    for (let d = 0; d < 4; d++) {
      const nx = cx + D[d][0], ny = cy + D[d][1];
      if (nx < 0 || ny < 0 || nx >= w || ny >= h || !passable(nx, ny)) continue;
      const n = id(nx, ny), ng = g[c] + 1 + (dir[c] !== -1 && dir[c] !== d ? TURN : 0);
      if (ng < g[n]) { g[n] = ng; came[n] = c; dir[n] = d; heap.push(n, ng + hz(nx, ny)); }
    }
  }
  return null;
}
```

Cache paths per `(from, to, layoutVersion)`. Recompute when the layout changes. Walk speed is 48 art px/s (Pixel Agents [V-src]), or 3 tiles/s. Facing comes from the step delta, using the dominant axis.

### 9.8 The 60 fps loop that drops to 0% CPU

```ts
// world/scheduler.ts
export class FrameScheduler {
  private raf = 0; private timer = 0; private visible = document.visibilityState === 'visible';
  private onScreen = true; private last = 0;
  constructor(private step: (now: number, dt: number) => { moving: boolean; nextStepAt: number },
              canvas: HTMLCanvasElement) {
    document.addEventListener('visibilitychange', () => {
      this.visible = document.visibilityState === 'visible';
      this.visible ? this.wake() : this.sleep();       // on wake: caller fast-forwards from Rust snapshot
    });
    new IntersectionObserver(([e]) => { this.onScreen = e.isIntersecting; this.onScreen ? this.wake() : this.sleep(); })
      .observe(canvas);                                  // World tab hidden / split collapsed → stop
  }
  /** Call on every reducer change, input (pan/zoom/hover), resize, theme change. */
  wake = () => { if (this.visible && this.onScreen && !this.raf) { clearTimeout(this.timer); this.raf = requestAnimationFrame(this.frame); } };
  private frame = (now: number) => {
    this.raf = 0;
    const dt = this.last ? Math.min((now - this.last) / 1000, 0.1) : 0;   // clamp like Pixel Agents
    this.last = now;
    const { moving, nextStepAt } = this.step(now, dt);   // simulate + render exactly once
    if (!this.visible || !this.onScreen) return;
    if (moving) { this.raf = requestAnimationFrame(this.frame); return; }  // walking/camera/fx → 60 fps
    this.last = 0;                                        // don't integrate across the gap
    if (Number.isFinite(nextStepAt))                      // looping poses: typing 300 ms, LEDs 250 ms…
      this.timer = window.setTimeout(this.wake, Math.max(4, nextStepAt - performance.now()));
    // else: nothing scheduled at all → 0 % CPU
  };
  private sleep() { cancelAnimationFrame(this.raf); this.raf = 0; clearTimeout(this.timer); this.last = 0; }
}
```

**Budgets** to put in CI perf tests and an Instruments script. These are targets, not measurements [U]:

| Situation | Target |
|---|---|
| No agents, or all idle | 0 wakeups/s, WebContent CPU < 0.3% |
| 3 agents typing (step timers about 3–8 Hz) | < 2% |
| Walking or panning (60 fps) | < 8% on an M1 |
| Frame time | < 2 ms for 24 agents + a 64×64 map |

**Notes.**
- WKWebView caps rAF near 60 fps even on ProMotion [R Apple forums]. That is fine for pixel art.
- **Persistent CSS animations also burn CPU.** Status pulses stop after 3 iterations. xterm `cursorBlink` runs only while the terminal is focused. There are no infinite spinners in chrome.
- Camera smoothing must be frame-rate independent: `cam.x += (tx - cam.x) * (1 - Math.exp(-dt * 10))`. Pixel Agents' fixed 0.1 per frame is not [V-src].

### 9.9 Pixel-perfect rendering

```ts
// Integer device-pixels per art pixel; no ctx.scale(dpr) (Pixel Agents approach [V-src]).
function resize(canvas: HTMLCanvasElement, cam: Camera) {
  const dpr = window.devicePixelRatio || 1, r = canvas.getBoundingClientRect();
  canvas.width = Math.round(r.width * dpr); canvas.height = Math.round(r.height * dpr);
  cam.zoom = clamp(Math.round(cam.zoomCss * dpr), 1, 10 * dpr);   // e.g. 3 CSS px/art px on Retina → 6
}
matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change', () => resizeAll()); // display change
// Tauri: also listen to WindowEvent::ScaleFactorChanged [V-doc]

function render(ctx: CanvasRenderingContext2D, w: World, cam: Camera) {
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.imageSmoothingEnabled = false;                          // reset after every context state change
  ctx.fillStyle = w.palette.void; ctx.fillRect(0, 0, ctx.canvas.width, ctx.canvas.height);
  const ox = Math.round(ctx.canvas.width / 2 - cam.x * cam.zoom);
  const oy = Math.round(ctx.canvas.height / 2 - cam.y * cam.zoom);
  ctx.setTransform(cam.zoom, 0, 0, cam.zoom, ox, oy);
  ctx.drawImage(w.staticLayer, 0, 0);                         // floor + walls(back) + rugs, prebaked OffscreenCanvas
  for (const d of w.drawables) {                              // pre-sorted (see z-rules)
    const x = Math.round(d.x * cam.zoom) / cam.zoom;          // snap to DEVICE pixels → smooth walking, no shimmer
    const y = Math.round(d.y * cam.zoom) / cam.zoom;
    ctx.drawImage(d.atlas, d.sx, d.sy, d.w, d.h, x - d.ax, y - d.ay, d.w, d.h);
  }
  w.overlay.style.transform = `translate(${ox / dpr}px, ${oy / dpr}px)`;  // DOM bubbles share camera
}
```

CSS: `canvas { image-rendering: pixelated; width: 100%; height: 100%; display: block; }`. Use `getContext('2d', { alpha: false })`.

**Z-sorting** (adapted from Pixel Agents [V-src]):
- Sort by `baseline` = the foot Y of a sprite or the bottom edge of furniture.
- Characters use `y + 8 + 0.5` so they draw in front of same-row furniture.
- Back-facing chairs use `(row+1)*16 + 1`, so the chair back covers the sitter.
- Surface items (monitor, laptop) use `max(spriteBottom, deskZ + 0.5)`.
- Walls are 16×32 sprites with a 16 px overhang, z-sorted like furniture.
- The selection outline is drawn just before its character (`z − 0.001`).
- Re-sort only when something moved (an insertion-sort pass is enough).

**Hit testing.** Convert the pointer to art space and walk drawables from top to bottom using the per-frame 1-bit alpha mask. Expand to a ≥ 24 CSS px target box (Vercel [V-doc]). Hover shows a tooltip: name · activity · since.

**DOM overlay.** Bubbles, name tags and the selected-agent ring label live in one absolutely positioned layer. During pan and zoom only that container's transform changes. Children move only when their agent moves. Positions are rounded to whole CSS pixels, which fixes the shimmer Munder also had to solve [V-src].

### 9.10 Camera, input, accessibility

| Input | Action |
|---|---|
| Pan | two-finger scroll, middle-drag, Space-drag, or arrows / WASD (when the canvas is focused) |
| Zoom | pinch or `⌘` + wheel, `+` / `−`, `0` = fit; integer steps 1–8× CSS |
| `F` | follow the selected agent |
| `Tab` / `⇧Tab` | cycle agents |
| `↵` | open the inspector |
| `Esc` | deselect |

- **Accessibility.** The canvas is `role="img"` with `aria-describedby` pointing to an off-screen `role="list"` of agents ("Ada, editing app.ts, 4 minutes"). That list is the real focus target, using roving tabindex synced to the selection. The event ticker is mirrored to an `aria-live="polite"` region, throttled to 1 announcement every 5 s, while approvals are announced immediately.
- **Reduced motion.** Walking becomes a crossfade-teleport. There is no confetti, rain or LED blink; static icons replace them. Typing and reading frames stay but never loop faster than 2 Hz.
- **Theme.** The world palette follows the app theme: light = daytime, dark = evening with warm lamps and stars in windows. This is done by palette swap at atlas build; no shaders.
- **Sound** (optional, off by default except approvals): a two-note chime. Pixel Agents uses E5 → E6 via WebAudio [V-src].

### 9.11 Replay

The world is `fold(worldReducer, snapshot@seqN, events(seqN..cursor))`, with the RNG seeded per agent and per activity id. Scrubbing renders the state at the cursor **without** animating the gap. Pressing Play animates forward at 1×, 4× or 16× (walk speed is scaled, and dwell rules are scaled too).

---

## 10. Recommendations for Codly (opinionated)

1. **Ship the world with zero third-party art.** Build `world/sprites` with procedural characters (§2.1) and about 25 code-defined furniture items (§2.2). Budget: 3–4 dev-days of pixel tuning. Add "Bring your own pack" (Pixel-Agents manifest schema) in v1.1 and never bundle LimeZu or Donarg.
2. **Use top-down 3/4 with 16 px tiles and a 16×24 character cell.** Treat isometric as a non-goal.
3. **Use Canvas 2D, not PixiJS.** WebGL brings context-loss recovery (Munder needed `glRecovery.ts` [V-src]), a 1 MB+ dependency, and nothing that ≤ 24 sprites need.
4. **Make the frame scheduler a hard requirement.** Add a perf test: idle world → 0 rAF callbacks over 10 s (spy on `requestAnimationFrame`).
5. **Rust owns truth.** Normalize to `WorldActivityEvent` in Rust (classification, test detection, risk hints) and persist it. The frontend reducer is pure and replayable. Native notifications for approvals come from Rust, because the webview may be suspended after about 5 minutes hidden [V-doc].
6. **Honesty contract** (§9.4): ≤ 250 ms to truth, 600 ms dwell, 400 ms / 1.2 s excursion rule, no wandering by default, provenance on everything.
7. **One Inspector for both views, with the four provenance badges.** This is the product's explainability moat; build it before the kanban and graph.
8. **Tokens.** Adopt §5.1 as-is: warm neutral h 75, cobalt accent h 255, generated from 3 inputs, contrast-checked in CI with WCAG and APCA. Bake OKLCH at build time; set `minimumSystemVersion: "13.0"`.
9. **Fonts.** `@fontsource-variable/inter` (opsz) and `@fontsource-variable/jetbrains-mono` 5.3.0, both bundled locally (no network). Optionally Geist Pixel for signage. Not Berkeley Mono.
10. **Motion.** Use the 100/150/200/240 ms ladder with ease-out-quint. Use the spring only on the palette and inspector. Honor `prefers-reduced-motion`. No infinite CSS animation in chrome.
11. **Keyboard map (v1):** `⌘K` palette (scopes `> @ # / ?`); `⌘N` new task; `⌘1/2/3` views; `[` sidebar; `⌘\` inspector; `G I/T/B/G/W` navigate; `J/K` lists; `↵` and `⌘↵` primary and secondary actions; approvals `⌘↵` / `⌥⌘↵` / `⌘⌫` / `⇧⌘⌫`; world `F`, `0`, `+`, `−`, `Tab`; `?` shortcut sheet. Implement with `tinykeys` 4.0.1 (MIT, supports sequences) [V-local].
12. **Split view by default on wide windows**, with a shared selection, hover and time cursor (§6.5). Workspace-only below 1280 px; World stays a `⌘2` away.
13. **Approvals in five places, toasts in none:** inbox, world, agent card, dock badge, native notification.
14. **Sanitize every world string** (§8.6). Treat agent output as untrusted display data.
15. **Label cost truthfully:** "≈ list price · included in plan" versus "billed (API key)". The context gauge is a snapshot.

---

## 11. Open questions and spikes (do these before building on the assumptions)

1. **WKWebView occlusion.** Does `visibilitychange` fire when the window is fully covered (not minimized) inside a Tauri window? Tauri 2.12.5 `WindowEvent` has no `Occluded` variant [V-local docs.rs]. If it doesn't fire, add a tiny Rust observer on `NSWindowDidChangeOcclusionStateNotification` (`objc2-app-kit` 0.3.2 [V-local]) and emit `codly://occluded` to the frontend.
2. **Emission of the newer Claude events** in plain `claude -p --output-format stream-json` versus only the SDK: `session_state_changed`, `tool_progress`, `tool_use_summary`, `task_*`, `compact_boundary`, `api_retry`. Run a fixture capture with a long Bash and a forced compaction.
3. **Gemini CLI `write_todos` tool name and fields** (0.62.0); this is unverified.
4. **Inter 4.1 `cv05` / `cv08` semantics**; check against the Inter feature docs or the font's `GSUB`.
5. **Berkeley Mono terms.** The vendor page returned 403; the decision only matters if someone insists on it.
6. **APCA thresholds for 12–13 px secondary text in dark mode.** Current Lc is about 63; consider bumping `--text-2` L to 0.80 if user testing finds it dim.
7. **DOM-overlay bubble cost with 24 walking agents.** If transforms show up in profiles, move bubbles into the canvas, rendering text at device resolution as Munder does [V-src].

---

## 12. Sources

**Code read today**
- Pixel Agents `pixel-agents-hq/pixel-agents` @ `b1bafd0`: `webview-ui/src/office/engine/{gameLoop,characters,renderer}.ts`, `office/layout/tileMap.ts`, `office/sprites/spriteData.ts`, `office/toolUtils.ts`, `src/constants.ts`, `CLAUDE.md`, `docs/external-assets.md`, `README.md`. https://github.com/pixel-agents-hq/pixel-agents
- Munder Difflin `HarnessMD/munder-difflin` @ `d3460de`: `LICENSE-ASSETS`, `src/renderer/src/assets/ATTRIBUTION.md`, `scene/office/{portraitArt,ToolBubble,MessageEnvelope,pathfinding,Character}.ts`, `DESIGN.md`, `package.json`. https://github.com/HarnessMD/munder-difflin and https://munderdiffl.in/
- Codex `openai/codex` @ `970111f`: `codex-rs/exec/src/exec_events.rs`
- Claude Agent SDK `@anthropic-ai/claude-agent-sdk` 0.3.310: `sdk.d.ts` (message unions and subtypes)
- Local fixtures (captured today by Codly research): `docs/research/fixtures/{claude-tool,claude-pong,gemini-stream-tool,acp-gemini,acp-opencode,opencode-run}.jsonl` (Claude Code 2.1.309, Gemini CLI 0.62.0, OpenCode 1.18.31)

**Docs**
- Linear, "How we redesigned the Linear UI (part II)" (2024-03-28): https://linear.app/now/how-we-redesigned-the-linear-ui
- Linear, "Behind the latest design refresh" (2026-03-12): https://linear.app/now/behind-the-latest-design-refresh
- Linear inbox / my-issues / changelog shortcut references: https://linear.app/docs/inbox , https://linear.app/docs/my-issues
- Vercel Geist colors / materials / typography: https://vercel.com/geist/colors , https://vercel.com/geist/materials , https://vercel.com/geist/typography
- Vercel Web Interface Guidelines: https://github.com/vercel-labs/web-interface-guidelines
- Raycast Toast / Action Panel / Keyboard: https://developers.raycast.com/api-reference/feedback/toast , https://developers.raycast.com/api-reference/user-interface/action-panel , https://developers.raycast.com/api-reference/keyboard
- ACP tool calls and plan: https://agentclientprotocol.com/protocol/tool-calls , https://agentclientprotocol.com/protocol/agent-plan
- Tauri: WindowEvent (2.12.5) https://docs.rs/tauri/latest/tauri/enum.WindowEvent.html ; WindowConfig (`background_throttling`, `background_color`, `title_bar_style`, `traffic_light_position`) https://docs.rs/tauri-utils/latest/tauri_utils/config/struct.WindowConfig.html ; macOS bundle minimum version https://v2.tauri.app/distribute/macos-application-bundle/
- MDN browser-compat-data (raw JSON): `css/types/color.json`, `css/types/easing-function.json`, `css/properties/image-rendering.json`, `api/OffscreenCanvas.json`, `api/Document.json`, `api/HTMLElement.json`
- Gather sizes and tiles: https://support.gather.town/articles/9253868124-sizes , https://support.gather.town/hc/en-us/articles/15910430385172
- Apple forums on the WKWebView 60 fps cap (Tauri): https://developer.apple.com/forums/thread/796512

**Licenses and assets**
- Kenney CC0: https://kenney.nl/support ; packs https://kenney.nl/assets/roguelike-rpg-pack , https://kenney.nl/assets/roguelike-indoors , https://kenney.nl/assets/roguelike-characters
- LimeZu: https://limezu.itch.io/moderninteriors , https://limezu.itch.io/modernoffice
- Donarg / RetroPi Office Tileset: https://retropi-art.itch.io/officetileset
- JIK-A-4 Metro City (CC0): https://jik-a-4.itch.io/metrocity-free-topdown-character-pack
- Little Bits: Office: https://adriccustoms.itch.io/little-bits-office
- Fonts: https://github.com/rsms/inter (4.1, OFL), https://github.com/vercel/geist-font (v1.8.2, OFL, includes Geist Pixel), https://github.com/JetBrains/JetBrainsMono (2.304, OFL); Berkeley Mono https://usgraphics.com/catalog (403 on the FX-202 page; terms via search snippet [R])

**Package versions checked today (npm / crates.io)**

| Package | Version | License |
|---|---|---|
| `pixi.js` (not chosen) | 8.22.0 | MIT |
| `tinykeys` | 4.0.1 | MIT |
| `@floating-ui/dom` | 1.8.0 | MIT |
| `@xterm/xterm` | 6.0.0 | MIT |
| `@xterm/addon-webgl` | 0.19.0 | MIT |
| `culori` | 4.0.2 | MIT |
| `motion` (not needed) | 14.6.0 | MIT |
| `@leeoniya/ufuzzy` | 1.0.19 | MIT |
| `virtua` | 0.53.6 | MIT |
| `@dagrejs/dagre` | 3.1.3 | MIT |
| `elkjs` (EPL/GPL; avoid) | 0.12.0 | EPL-2.0 OR GPL-3.0-or-later |
| `lucide` | 1.61.0 | ISC |
| `@fontsource-variable/{inter,jetbrains-mono,geist,geist-mono}` | 5.3.0 | OFL-1.1 |
| `geist` | 1.8.2 | OFL |
| `solid-js` (if a framework is wanted) | 1.9.17 | MIT |
| `tauri` (crate) | 2.12.5 | |
| `@tauri-apps/api` | 2.12.4 | |
| `window-vibrancy` (crate) | 0.8.1 | |
| `objc2-app-kit` (crate) | 0.3.2 | |
| `easystarjs` / `pathfinding` (stale since 2022; don't use) | 0.4.4 / 0.4.18 | |
