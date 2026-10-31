# Progression: from a garage to a neighbourhood

Status: plan (2026-10-30). Builds on `SIMS.md` (careers, moodlets, Build/Buy).

A new crew starts broke, in a garage, with the strict minimum. Real work earns money; money
buys better buildings and furniture. Each repo is an office on a lot in a small 3D
neighbourhood: you buy and sell buildings, and move agents between your offices. Agents get tired over a shift of real work and recover by going
home for a few minutes, not with a one-second coffee. Coffee exists but has a cost. An agent that sticks around for months
becomes a legend: barely tires and holds its focus. Later, offices can be visited and compared
from the web app.

## 1. What exists today

- **XP** per agent (`career.rs`): commit +5, passing check +10, approved review +20, review done
  +10, merge +30. Ranks: Junior 60, Senior 250, Staff 800.
- **Office budget** (`orchestrator/careers.rs::shop`): the sum of every agent's XP in the repo,
  ghosts included, minus what was bought.
- **Build/Buy**: 139 catalog pieces (`furniture.json`), sold back at full price.
- **The floor plan is fixed** (`layout.ts`): 9 rooms, 24 desks, a full kitchen and a games
  corner, all open from day one.
- **Needs** (`sims.ts`):
  - Focus is real: the context window left.
  - Energy is simulated. It drains while working, and a minute of coffee or a nap tops it up.
- **Rooms do real work**: tests go to the server room, reading to the library, messages to the
  mail room, memory to the archive (`behavior.ts::stationFor`).

## 2. The rules

1. **Feelings come from real events** (`SIMS.md`). Where a need can be real, it is:
   - Focus is the context window. It is managed as it is today, not by the game:
     the Lead picks `continue` or `fresh` when it hands off work (fresh for unrelated work,
     or above ~70% context), and the CLI compacts by itself when it runs out of room. The
     timed "compact every 2 h" rule is removed. Going home does not touch the context.
   - "Out of quota" is the real usage limit.
2. **The game can say no; you can always overrule it.**
   - An agent at home gets no new work from the Lead, but you can call it in.
   - A locked room changes where an agent *appears* to work, not whether it can.
   - The game never stops the crew from shipping. If everyone is home, the Lead calls back the
     most rested agent.
3. **Cheats are marked.** Developer grants flag the office for good (it matters for leaderboards).

## 3. Phase 1: money, and a cheat for testing (1–2 days)

- **Funds, separate from XP.** XP is the career; funds are what the office spends.
  Funds = XP earned + grants − spent; the ledger is `shop()` plus `grants:{ws}`. Shown as coins
  (🪙), not `$`, which already means real API cost.
- **Developer panel** in Settings, behind a "Developer mode" toggle:
  - Add 🪙 1,000 / 10,000.
  - Give an agent XP (next rank).
  - Fill or drain an agent's energy.
  - Skip time (home and caffeine timers).
  - Reset this office to the garage (asks first).

## 4. Phase 2: lots, build mode and the neighbourhood (3–4 weeks)

Like a life-sim neighbourhood: you own lots, each lot has one office, and **each office is one
repo**. You build and rebuild the office on its lot as the crew earns. When it outgrows the lot,
you buy a bigger lot and move:
- the crew carries boxes to a van;
- what doesn't fit goes to storage;
- the old lot stays yours until you sell it (80% back, furniture 100%) or put another repo on it.

### The map

- **A 3D neighbourhood view**: flat plain ground and a grid of lots, nothing else for now (roads,
  trees and weather later). Same camera controls as the office.
- **Each owned lot** shows its building from outside, roof on, with a sign: the repo's name and
  its crew count. Lights are on when its agents are working.
- **Click a lot to enter it**: this opens that repo, as "open workspace" does today. The office
  view is the inside of the lot.
- **Empty lots** show "For sale · 🪙 N".
- **Every office with work is live.** The core keeps scheduling each repo with a running plan
  or a task in flight, whichever one you look at; worker slots and account caps count all of
  them together. Those lots are lit with "Working · N"; idle ones show "Closed". Office tabs at
  the top of the rail hop between them (closing a tab only hides it).

### Lots and build mode

You don't pick a floor plan; you build one. A lot is an empty plot of land, and **build mode**
lets you put up walls, paint them, lay floors and place doors and windows, as in a life sim.
A small lot can hold a lot of office if you build it well.

**Lots for sale**, each sold empty or with a ready-made building on it:

| Lot | Size | Empty | Ready-made |
|---|---|---|---|
| Small | 10 × 8 m | 🪙 300 | **Garage** (your first one is a gift): roll-up door, Lead + 2 desks, server machine, bookshelf, mailbox |
| Medium | 16 × 12 m | 🪙 1,200 | **Small office**: a work floor and a back room, ≈ 🪙 2,000 |
| Large | 28 × 24 m | 🪙 4,000 | **Studio**: today's 9-room floor plan, ≈ 🪙 9,000 |

**A ready-made building is just a saved design, and everything in it is yours like anything you
built.** You can:
- move, turn, sell or delete any piece of furniture;
- knock down, move or repaint any wall, door, window or floor;
- empty it completely and start over.

**Prices can't be gamed.** Every price comes from one table in the core, and there is no way to
buy something and sell it for more than it cost:

- **A ready-made lot costs exactly** the empty lot plus everything on it, at the same prices as
  building it yourself (walls per metre, doors, paint, each piece of furniture). This total is
  computed from the design, never typed by hand; the ≈ prices above are just estimates.
- **Everything sells back at one rate per kind**, whether you placed it or it came with the lot:

  | Kind | Sells back for |
  |---|---|
  | Furniture | 100% |
  | Walls, doors, windows | 50% |
  | Paint and floors | 0% |
  | The lot itself | 80% |

  So buying a ready-made lot and selling it piece by piece always returns at most what you paid.
- **Gifts sell for nothing.** The first garage and the free Studio for existing repos are gifts.
  Their pieces can be moved, stored or deleted, but they sell for 🪙 0 and carry a small "gift"
  tag. Otherwise a new player could sell the starter garage for coins out of nothing.
- **Cheat grants** (developer panel) flag the office, as in §2.
- **Tests guard it**:
  - for every ready-made design, price = lot + sum of its contents;
  - for any sequence of buy, sell, move and rebuild, coins after ≤ coins before, unless real
    work earned some in between.

**Build tools** (a toolbar in build mode, each with a live price):

| Tool | What it does | Price |
|---|---|---|
| **Room** | Drag a rectangle: four walls and a floor in one go | walls + floor |
| **Wall** | Drag along the grid lines (1 m, or 0.5 m with Alt). Solid, glass, or half-height | 🪙 10 per metre |
| **Push / pull** | Grab a wall's arrow and drag it: the room grows or shrinks. Furniture that no longer fits goes to storage | the difference |
| **Move room** | Pick up a whole room, with what's in it, and put it down elsewhere or turn it | free |
| **Door** | Put a door in a wall: single, double, glass. Every room needs a way in | 🪙 40–120 |
| **Window** | On outside walls, anywhere along them | 🪙 30–90 |
| **Floor** | Paint a whole room or tile by tile: oak, walnut, carpet, tile, terrazzo, concrete | 🪙 1 per m² |
| **Wall paint** | A colour or wallpaper on one side of a wall, or a whole room's inside | 🪙 1 per m² |
| **Sledgehammer** | Delete a wall, door, window or room; pays back 50% | — |
| **Eyedropper** | Copy a paint, floor or piece | — |
| **Undo / redo** | ⌘Z / ⇧⌘Z for every build action | — |

- **Rooms are found, not declared**: an area closed by walls is a room. Its floor fills in, and
  it's named after what's inside: racks make a "Server room", shelves a "Library". You can rename
  it.
- **Work stations are furniture.** Server racks, bookshelves, the mailbox, memory cabinets and
  desks are catalog pieces with a job. Each desk is a seat for one agent.
- **Every office needs** a front door, a desk for the Lead, a server machine, a bookshelf and a
  mailbox, all reachable from the door. Build mode shows this checklist and won't let you remove
  the last one of any of them, so the work always has somewhere to go.
- **Save a room to your library** and drop it into another office; later, share rooms and whole
  lots through the web app (Phase 7).
- **Walls up / cutaway / down** while building, as the office does today.

**Code**

- **A lot becomes data**, `lot:{ws}`: `{ size, walls, doors, windows, floors, paint, furniture }`.
  `layout.ts` turns it into the same `OfficeLayout` it builds by hand today, so the engine and
  the walking grid don't change.
- **The Studio and the Garage become two saved lots (JSON)**. The Studio's file is made once from
  today's `build()`.
- **Walls stay axis-aligned** (no diagonals or curves) in the first version. `building.ts`
  already draws walls as segments with gaps and windows, and the walls-down modes.
- **Floors move from one material per room rectangle to per tile** (grouped by material), so
  that painting and odd-shaped rooms work.
- `buy.ts` keeps its placement rules (grid, collisions, door reachability) and adds walls and
  doors as obstacles from the lot data.

**Order inside this phase**

1. Lots as data, with the Studio and Garage templates loaded from JSON (no editor yet).
2. The Room, Wall, Door, Window and Sledgehammer tools, the checklist and undo.
3. Floor and wall paint.
4. Push/pull walls, move room, and the room library.
5. The neighbourhood map and buying, selling and moving lots.

### Your crew across offices

- **Agents belong to you**, not to a repo. **Transfer** an agent to another of your offices: it
  walks out the door here and turns up at the other one.
  - Its career, traits, skills and stamina travel with it.
  - Trust and friendships stay behind, as they already do for an exported agent.
  - It can't transfer mid-task.
  - The core moves the agent's row to the other repo instead of exporting and importing it, so
    it keeps its id and history.
- **A new repo** starts as an empty lot: buy a garage there, or move an office in.

### Money

- **One wallet per player** (🪙). Every office earns into it and every purchase spends from it,
  as for a company with several branches.
- **Existing repos** each get a Studio on a lot, for free and fully furnished, so nobody's
  office shrinks after an update. New repos start with a garage.

## 5. Phase 3: tired, home, back (3–4 days)

Energy becomes a day-scale need instead of a minutes-scale one.

- **Drains with real work**: about one shift (~4 h of real working time) from full to empty for
  an Intern, slower with stamina (§7). Idle time drains a little; waiting for you does not.
- **Low energy**:
  - Under 30% it is **Tired**: morale slides, it walks slower, and the Lead prefers rested agents
    for new tasks.
  - At 0% it is **Exhausted**: it finishes its current task, then goes home by itself.
- **Going home** (a button on the agent, or by itself when exhausted):
  - It walks out the door, and its name tag reads "🏠 Home · back in 6 min".
  - It gets no new tasks; its conversation is kept as it is.
  - Energy refills in **a few minutes**: ~10 min for an Intern, ~3 min for a Legend.
  - It comes back by itself, or you can call it in early: **Overtime**, morale −0.15, half the
    energy it had banked.
- **The Lead** never goes home (you talk to it). It tires like the others, but only the office
  shows it: yawning, slumped at its desk, morale down.
- **Napping on a sofa** (once you own one): rest at a third of home speed.

## 6. Phase 4: coffee and getting sick (1–2 days)

A coffee machine (🪙 120) needs a kitchen corner: Small office and up, never the garage.

- **A cup** gives +15% energy right away and adds a cup of caffeine, which wears off over ~1 h.
- **Jittery** (3+ cups): morale −, it fidgets at its desk.
- **Sick** (5+ cups since it last went home, or 2 shifts in a row without going home):
  - It must go home for a long rest (~30 min).
  - Energy is capped at 60% for its next shift.
  - Moodlets: 🤒 "Sick: too much coffee, not enough sleep".
- Tolerance grows with stamina: a Legend can take 6 cups before it gets jittery.

## 7. Phase 5: stamina and legends (2–3 days)

**Stamina** (0–100) is built from what an agent has really lived through. It grows with real
days on the crew, shifts completed, and merges, and drops a little after being sick. It scales:

| | Intern, stamina 0 | Legend, stamina 100 |
|---|---|---|
| Energy drain per working hour | 25% | 8% |
| Time at home to recover | ~10 min | ~3 min |
| Coffee before jittery | 2 | 6 |
| Shifts before sick without going home | 2 | 4 |

- **Legendary** is a title above Staff. It needs Staff rank, 90 days on the crew, 150 merges and
  stamina ≥ 80, so it can't be cheated out of XP alone.
  - Its name tag turns gold, it gets a "Legend" milestone, and its plumbob glows.
  - At promotion the app offers it the biggest context window available, so its real focus grows
    too (you decide, as for ranks today).
- A legend leaving the crew becomes a famous ghost, and bringing one back is an event.

## 8. Phase 6: things worth earning, and bills (later)

- **The games corner and comforts move into the shop**: console (PS5), foosball, ping-pong, pool,
  sofa, TV.
  - Some need a rank: "Someone on the crew must be a Senior".
  - Agents only use what you own.
- **Bills from real cost** (optional): weekly, the office pays its real API spend in coins, so a
  productive, cheap crew gets rich.

## 9. Phase 7: visiting and competition (web app)

1. **Export a snapshot** (`office.json`): the lot (walls, paint, furniture), and the agents' names, looks, ranks,
   stamina and public stats. No code, paths or prompts. It builds on `export_agent`.
2. **A read-only 3D viewer** in the web app, with the same `engine.ts`. Other players' offices
   appear as lots at the edge of your neighbourhood map, and you can walk in.
3. **Publish and visit**: GitHub sign-in, shareable links, a guestbook.
4. **Leaderboards**: weekly merges, XP, cost per merge, number of legends. Cheated offices are not
   ranked; an office that links its public repo gets verified merges.
5. **Competitions**: weekly challenges, office vs office; trophies for the lobby.

Phases 1–6 are local; Phase 7 needs a backend.

## 10. Still to think about

- **Lot layout.** A fixed grid of lots (simplest), or free placement on the plain.
- **Hiring costs.** Hiring is free today. A signing fee in coins, or a daily salary by rank,
  would make each hire a decision and gives coins a steady sink. A salary can't stop work: an
  unpaid crew just gets a "Late pay" moodlet.
- **No coin farming.** Coins only from work that lands: a merge pays only once its checks
  passed, commits and reviews have a daily cap per agent, and a reverted merge takes its coins
  back.
- **Day and night.** Office lights follow your real clock; agents with the Night owl trait
  tire slower at night. Weekends: the crew could take Saturday off unless you call them in.
- **Wants**, Sims-style: each agent gets one small goal from real work ("merge 2 PRs today",
  "review someone's code") that pays a bonus and a good moodlet.
- **Getting sick spreads.** Calling in a sick agent can make a friend at the next desk sick.
- **Quitting.** A long stretch of low morale could make an agent ask to leave (you choose);
  it would never just stop working.
- **Zen mode.** A switch for people who only want the tool: no tiredness, no coffee, no
  prices, every building free. The game is opt-out, never in the way.
- **The browser demo** must show the climb too: start in the garage, then buy a small office
  and move in during the story.
- **Onboarding.** The first-run quest ends in the garage, with a short tour: "This is where
  it starts."
- **Save and reset.** Export an office (Phase 7) is also a backup; "Reset to garage" in the
  developer panel is the way to replay.
- **Sound.** Each building gets its own sounds: an echoing garage, a busy studio.

## 11. Decisions

1. **Separate funds** from XP (recommended).
2. **Existing repos** get a free Studio, fully furnished (recommended), or start in a garage.
3. **Desks are a soft limit**: extra hires work from a crate or the lobby bench (recommended).
4. **One wallet per player** (recommended), or one per office.
5. **At home means no new tasks unless you call it in** (recommended), or the Lead may call one
   in on its own when everyone else is busy.
6. **Timings**: a shift is about 4 h of real working time for an Intern, and recovery at home takes
   minutes (tune both after a week of play).
