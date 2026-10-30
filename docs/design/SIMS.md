# The office, closer to The Sims (plan)

> Brief (2026-10-27): idle characters move weirdly; at the foosball table and the PS5 they all do
> the same twitch instead of really playing. Move the office **much** closer to The Sims, in look,
> in UI/UX, and later in behaviour: agents with personalities and flaws, "more human, like a game".

## 1. Why it looks wrong today

`src/office/gfx/character.ts` drives a 14-number rig: body bob, torso pitch/yaw, head (3 axes),
each shoulder (2 axes) and elbow, each leg as one rigid piece. There are no knees, feet, hands,
wrists, spine or hips.

Every activity is a **stepped loop**: the joints jump between two fixed angles every 150–600 ms
(`s % 2 ? a : b`) with a 240 ms blend. So:

| What you see | Why |
|---|---|
| PS5: a tiny elbow flick, forever | `gamepad` toggles the forearm by 0.06 rad every 300 ms |
| Foosball and pool look identical | both are the same `play` pose: a ±0.14 rad torso twist every 450 ms |
| Idle agents are statues, then twitch | `stand` / `sit` are fully static by design ("a character standing still costs nothing"), loops are binary flips |
| Sitting looks stiff | a sit is a whole leg rotated 86° at the hip: no knee, so shins stick forward |

The cause is the animation model, not the tuning. The Sims looks alive because of
(a) continuous motion with overlapping layers (breathing, weight shifts, blinks, glances),
(b) object-specific animations that line up with the object (hands on the rods, controller in
both hands, eyes on the TV), (c) transitions (sit down, stand up, turn before walking), and
(d) reactions (cheer, groan, slump) at moments that mean something.

## 2. The rule that does not change

The world stays honest. **Work** is only shown when it happens: a character types because its
agent is editing, runs to the server room because a command is running, waits at your door
because an approval is pending (`behavior.ts` honesty contract, ≤ 250 ms to truth).

What we add is **life around the work**, in the gaps the truth leaves open: breathing, glances,
fidgets, idle autonomy, leisure, personality, and reactions to real outcomes. Flaws stay in that
domain: an agent can lose at FIFA, miss a foosball shot or spill its coffee after a failed test;
it never appears to do work it did not do, and never hides work it did.

Three layers, each allowed to override the one below:

```
Truth  (behavior.ts)   where the agent is and what it does, from real events      wins always
Life   (new: life.ts)  how it does it: idle autonomy, traits, needs, reactions    yields in ≤ 250 ms
Motion (new: anim/)    clips, layers, IK, transitions, look-at                    renders it
```

## 3. Phase 0: fix the motion on today's rig (days)

The complaint, fixed without new assets.

- **Continuous micro-motion** for every visible character: breathing (torso/shoulders),
  slow weight shift when standing, blinks (the closed-eyes head already exists), head glances at
  nearby events and other characters (a look-at target, smoothed).
- **Replace binary flips with curves**: each loop becomes a short keyframed cycle with eased
  in-betweens and per-agent phase and tempo, so two typists never move in sync.
- **Separate the games**:
  - *Foosball*: two players face each other at the rods. A simulated ball moves on the
    table; each player's rods twist when the ball is in their half, with a lean and a quick
    wrist snap on shots; goals trigger a cheer or a groan; a match lasts a few minutes.
  - *Pool*: walk around the table to a shot spot, crouch, line up the cue (slow draw back), shoot,
    watch the ball, then the other player's turn.
  - *PS5*: sit back on the couch, controller in both hands, thumbs moving, body leaning with an
    intensity curve, occasional "almost!" lean-forward and a win/lose reaction; two players
    glance at each other.
- **A frame budget instead of zero frames**: animate at up to 30 fps while the office is on
  screen and the window is focused; drop to today's stepped mode when unfocused or in Low Power
  Mode; 0 frames when hidden. Keep the "Measure idle" harness button to check the cost.

## 4. Phase 1: a real rig and animation system (1–2 weeks)

**Rig.** Keep the procedural, seed-based look (identity colours, hair, role silhouettes,
portraits) but build it on a humanoid skeleton with hips, spine, chest, neck, head, shoulders,
upper/lower arms, hands, thighs, shins and feet. Each body part stays a rigid low-poly piece bound
to one bone (segmented skinning), which suits the chibi style and costs almost nothing.

**Clips.** Use three.js `AnimationMixer` with layers:

| Layer | Examples | Source |
|---|---|---|
| Base | idle stand, idle sit, walk, start/stop/turn, sit down, stand up | CC0 libraries retargeted to our bone names |
| Upper body | type, read, phone, write, gamepad, rods, cue, mug | our own clips (code-authored keyframes or Blender) |
| Additive | breathing, weight shift, fidgets | our own, small |
| Procedural | head look-at, hand IK onto rods, controller and keyboard; foot planting | code |

CC0 candidates for the base layer: **KayKit Character Animations** (150+ humanoid clips including
idling, interacting, waving, cheering, sitting and lying down; glTF and FBX) and **Quaternius
Universal Animation Library** (120+ clips including sitting; free Standard tier; glTF). Neither has
foosball or gamepad clips, so those stay ours.

**Objects get slots** like Sims objects: an approach point, a facing, an animation set
(enter / loop / exit), and how many users. Desks, chairs, couch, foosball (2), pool (2),
coffee machine, whiteboard, bookshelves, server racks.

**Locomotion**: turn in place before walking, accelerate/decelerate, avoid other characters
(local avoidance on top of A*), queue politely at a busy slot.

## 5. Phase 2: Sims UI/UX, mapped to real data (1–2 weeks)

| The Sims | In Codly | Real data behind it |
|---|---|---|
| Plumbob over the selected Sim, coloured by mood | plumbob over the selected agent, coloured by state | working / waiting for you / blocked / rate-limited |
| Action queue (top-left icons, ✕ to cancel) | the agent's queue: current tool call, queued prompts | live run; ✕ = interrupt or stop |
| Pie menu on click | Chat 1:1, Show task, Show terminal, Change model, Stop, Send on a break | existing commands |
| Needs bars | **Energy** = quota left on its account · **Focus** = context window left · **Morale** = recent checks and reviews | `UsageWindow`, `Usage.context_used`, gate results |
| Moodlets ("Inspired", "Tense") | "Tests passed", "Waiting on you", "Rate-limited", "Merged", with timers | events |
| Thought bubbles | an icon of what it is thinking about (file, test tube, question) | reasoning / activity |
| Simlish speech bubbles | agent ↔ Lead messages as bubbles with icons; click to read the real text | `codly_ask_lead`, notes |
| Skills | TypeScript, Rust, Tests, Reviews… growing with real history | files changed, checks passed |
| Relationships | who reviews and asks whom | review and ask events |
| Notifications | Sims-style cards for approvals, merges and failures | existing toasts |
| Money counter | session cost | `cost_usd` |
| Speed controls | live: Pause = stop starting new tasks; replay: 1×/2×/3× through past events | scheduler pause (new), event log |
| Camera | walls up / cutaway / down, 90° rotations, zoom presets, follow the selected agent | camera.ts |
| Time of day | lighting follows your local clock | — |

## 6. Phase 3: personality and flaws (1–2 weeks)

Each agent gets two or three **traits** at creation (seeded, shown in its sheet). Traits only
change *how* it lives and reacts, triggered by real events or idle time:

| Trait | Idle and leisure | Reaction to real events |
|---|---|---|
| Clumsy | trips on the rug now and then | knocks its mug over when a command fails |
| Hot-headed | plays foosball too hard | thumps the desk when checks fail; storms to the coffee machine |
| Perfectionist | straightens things on its desk | rubs its temples on "changes requested", re-reads |
| Lazy | stretches and yawns; first to the couch | slumps during long runs |
| Social | chats at the coffee machine | high-fives whoever it reviewed |
| Night owl / Early bird | energy follows your local clock | yawns, or perks up |
| Competitive | trash-talks at the games | fist pump when its task merges first |
| Bookworm | reads in the library when idle | — |

**Idle autonomy** (Sims "free will", only when the truth layer leaves the agent idle): pick
an activity by needs and traits (coffee when Energy is low, games when Morale is low, chat with
the agent it works with most), walk there, and come back the instant real work arrives.

**Needs drift** from real telemetry (section 5), so a heavily used account really does look tired.

## 7. Later

Build mode (rearrange and decorate the office), more rooms and objects (kitchen, plants),
per-agent desks that fill up with things from their history (a trophy per merged plan).

## Status (2026-10-28)

| Phase | Built | Not yet |
|---|---|---|
| 0 | Continuous life (breathing, blinks, glances, fidgets), smooth per-agent loops, real foosball / pool / PS5 simulations, 30 fps while on screen (focused or not) | Shadows don't follow breathing or rolling balls |
| Break room | `breakroom.ts`: pairs for pool and foosball, PS5 solo or two-player, stable pairs, a lonely player moves on after 6 s; library and coffee spots; demo `?play=` and `?playAfter=` | |
| 1 | Skeleton: pelvis sway, thighs, knees, ankles, wrists, seat height as a joint; real gait; sitting down and getting up through a crouch. KayKit Character Animations (CC0) clips, converted to our joints by `scripts/kaykit-clips.py` into `gfx/clips.json`: Cheer (celebrate after passing checks), Dance (a merge, a promotion), Wave (hello at the door, the social trait's reaction) | KayKit's rig has no legs (legs keep our own motion); Quaternius not imported; more clips (Interact, PickUp) are one line in the script |
| 2 | Plumbob, action queue (✕ interrupts), click menu (chat, task, details, model, stop, send home), Needs / Mood / Skills / Friends panel, speech bubbles, estimated cost counter, walls up/down, **Pause** (core: no new task starts) | Replay at 1×/2×/3×; skills and friendships reset when the app restarts (not saved yet) |
| 3 | Traits (`traits.ts`), break timing and choices by traits and needs (tired → coffee, down → couch, bookworm → library), personal reactions to real check results, lazy fidgets, pool skill | |

| Careers | `career.rs`: XP from real commits, passing checks, approved reviews and merges; Intern → Junior → Senior → Staff; skills per area; milestones; saved on the agent and shown in its panel (rank, XP, Career tab); a promotion offers a stronger model | |
| Trust | A promotion to Senior or Staff offers trust; you grant it (promotion toast or Career tab) and can take it back. Trusted Senior: works autonomously in its worktree where you chose Balanced (risks still ask; Strict and Full stay as you set them). Trusted Staff: its tasks skip the cross-vendor review. Never above its rank; an imported agent starts untrusted (`career::PERKS`, `approvals::with_trust`) | No Junior perk yet |
| Hiring | A new consultant's job offer is pinned by the front door for 30 s with 3 applicants; you pick one or the Lead's pick is hired. The add-agent dialog starts with applicants and a trait picker (Create-a-Sim). Four traits change how an agent works (their rule rides on its prompts) | Applicants don't walk into the office yet |
| Export | "Export agent" saves `codly/agent@1` (manifest, traits, career, memory file); importing it in any repo brings the same colleague back | Solo agents don't get their memory file in their prompts yet |
| Ghosts | Consultants let go keep their career and float, see-through, by the memory archive (up to 3); the tag there lists them with their record and scrapbook; Bring back makes one a regular worker again (the scheduler reuses it, you can assign it) | Team members and Leads come back their own way (Settings, threads), so they are not ghosts |
| Scrapbook | Every milestone (joined, first commit, first merge, every 10th merge, promotions, moved from another repo, came back) by day in the Career tab and for ghosts | |
| Build/Buy | The XP every agent earned in the repo (ghosts too) is the office budget. A catalog of 126 pieces in 9 categories (`src/office/furniture.json`, CC0 models in `public/furniture`) with thumbnails; carry a piece, turn it (R) and put it down anywhere it fits: it snaps to the walking grid, may not cover walls, furniture, seats or the door, nor cut a seat off from the door (`buy.ts`). Lamps, plants and screens go on a table you bought. Click a piece to move or sell it (full price). Agents path around what you place; saved per repo | No wall-hung pieces (posters, upper cabinets); small things go only on bought tables, not built-in desks; an agent already walking keeps its path for that walk |
| Furniture you can use | The office's own furniture is drawn with the same CC0 models (`gfx/builtins.ts`, fitted to the plan's spots, fabrics recolored to the office palette). Click a piece for its Sims menu (`FurnitureMenu.svelte`): its uses (Sit, Nap, Read, Get a coffee, Watch TV, Dance, Play…) and the agents free between tasks to send; real work calls them back. On breaks agents also pick bought pieces (`use.ts`, `breakroom.ts`): a bookcase when the library is full, a sofa to nap, half of them lounge on a sofa or by a TV or radio rather than play | Live pieces stay code-built (desk monitors, racks, boards, mail room, foosball, pool, the TV picture); no free server rack or pool table model found |

## 8. Decisions

1. **Rig**: upgrade the procedural characters to a skinned humanoid rig (recommended: keeps
   identity and portraits, unlocks CC0 clip libraries), or switch to ready-made CC0 characters
   (KayKit / Quaternius style), or commission custom art.
2. **Frame budget**: accept up to 30 fps while the office is visible (needed for Sims-like life),
   replacing "zero frames when nothing happens".
3. **Flaws**: confirm they stay cosmetic and in leisure and reactions, never in the work itself.

## Sources

- KayKit Character Animations 1.2 by Kay Lousberg (CC0), [opengameart.org](https://opengameart.org/content/kaykit-character-animations): the Cheer, Dance and Wave clips
- Furniture Kit 2.0 by Kenney (CC0), [kenney.nl](https://kenney.nl/assets/furniture-kit), and KayKit Furniture Bits 1.0 by Kay Lousberg (CC0), [GitHub](https://github.com/KayKit-Game-Assets/KayKit-Furniture-Bits-1.0): the Build/Buy catalog

- [KayKit Character Animations (CC0)](https://kaylousberg.itch.io/kaykit-character-animations)
- [Quaternius Universal Animation Library (CC0)](https://quaternius.com/packs/universalanimationlibrary.html)
- [80.lv on the Universal Animation Library](https://80.lv/articles/get-your-hands-on-over-120-ready-to-use-game-animations)
- [SimGuruGraham on the Sims 4 interaction queue (2014)](https://simscommunity.info/2014/07/20/the-sims-4-simgurugraham-clarifies-the-new-video/)
- [The Sims 4 pie menu update (2023)](https://simscommunity.info/2023/02/27/the-sims-4-pie-menu-update/)
