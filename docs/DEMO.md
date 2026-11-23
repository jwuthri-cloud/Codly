# The browser demo

`pnpm dev` without the Tauri shell serves the UI on <http://localhost:1420> with a scripted
backend: no agent CLI runs and nothing is written to disk. Use it to look at a screen without
building the app. The script is in `src/lib/demo/story.ts`, and `src/lib/mock.ts` answers every
command the UI sends (`src/lib/__tests__/commands.test.ts` fails when one is missing).

The story: you ask for a dark-mode toggle on the repo `acme/web`, add a detail while the Lead
plans, and open a second request a little later. Three agents build the first plan, two build the
second. Codly checks and merges each task, and each Lead reports in its request.

Parameters go in the URL and can be combined. They are read once, when the page loads.

## The script

| Parameter | What it does |
| --- | --- |
| `?ws=1` | Opens `acme/web` at once. |
| `?speed=2` | Plays the script faster (`0.5` is slower). |
| `?skip=90` | Starts 90 seconds into the script. Any value, `0` included, also opens the repo. |
| `?autoplay=1` | Runs each plan and allows each request by itself. |
| `?onboard=1` | A machine with no agent CLI installed, to see the first-run quest. |

`?autoplay=1&skip=180` is the short way to both plans finished.

## States the script does not reach

Each of these opens `acme/web` at once, except `?trust=ask` and `?hour=`.

| Parameter | What you get | Where to look |
| --- | --- | --- |
| `?trust=ask` | The repo has CLI settings of its own, and you were never asked about them. | Open `acme/web`: the trust dialog. |
| `?merge=conflict` | Your branch changed a file that each finished plan changed too. Merge is refused. "Have an agent resolve it" gives the plan one more task; when it is done, the plan merges. | The plan's finished card: `?merge=conflict&autoplay=1&skip=180` |
| `?preview=running` | A command you started earlier runs in each worktree you look at. | "Try it before you merge" on a finished plan's card (`?preview=running&autoplay=1&skip=180`), or on the Git tab of a 1:1 agent. |
| `?intake=issue`, `?intake=pr` | The "From issue…" (or "From pull request…") dialog is open, and the scripted requests do not start: yours is the first. Any number works. "Run plans without asking" is on in this state, and the plan waits all the same, because its request started from GitHub: its card says why above "Run plan". Run the plan, then "Create pull request…": the description ends with `Closes #N`. | The dialog, then the request it opens. |
| `?helper=slow` | The helper (sub-agent) that Ada starts in her first task works for about a minute, not three seconds. | Ada's work in the first request: `?helper=slow&autoplay=1&skip=20` |
| `?checks=on` | The repo's required checks are on, so Codly runs them for each task after the task's own checks. | A finished task's sheet, under its checks ("required by you"): `?checks=on&autoplay=1&skip=180` |
| `?mode=plan` | Eli, a 1:1 Claude agent in the mode "plan first", was asked for a small change and shows a plan. Send it back with a note and it comes again; accept it and Eli asks before the edit. | Eli's conversation; the mode switch is in Eli's sheet, under Controls. |
| `?resume=ask` | Codly "closed" while the first plan ran: two of its tasks were in work and wait, and nothing starts. The bar at the top asks. "Resume work" starts them; "Not now" keeps them waiting, with the button; cancel a waiting task and the bar counts one less. (`billing-api` plays its own script: it is not held.) | The bar at the top of the window, and the two tasks on the board. |
| `?background=on` | Eli, a 1:1 Claude agent, ended its turn with two things at work in the background: a dev server and a helper. "Stop" ends the one you press it on. After the helper's stop Eli says so in a short turn of its own, as Claude Code does; after the command's stop no turn follows. Left alone, the helper reports after about three minutes. | Eli's sheet, under "In the background". |
| `?hour=13` | The demo's clock says 13:00 when the page loads, and goes on from there (from `0`, below `24`; `21.5` is 21:30). The office and the map are lit for the hour of the clock, so this sets their light; every time on screen follows. It opens no repo. | The office and the map: `?hour=13&ws=1` with the app's dark theme (a bright office under a dark app), `?hour=23&ws=1` with its light theme. |

### A kind of request (`?approval=`)

The script asks before commands, and one question with two short options. Each value below makes
a 1:1 agent ask another kind of request once, a few seconds after the page loads. One value for
each page load. Approve, deny or answer as on the other cards: the agent goes on from your answer.

| Parameter | Who asks what | Where to look |
| --- | --- | --- |
| `?approval=file` | Eli (Claude Code, "ask for edits") asks before it changes `README.md`, and shows the change. | Eli's conversation. |
| `?approval=plan` | Eli (Claude Code, "plan first") shows its plan. The same as `?mode=plan`. | Eli's conversation. |
| `?approval=network` | Ned (Claude Code): a command in Claude Code's sandbox wants to reach `registry.npmjs.org`. Gil (Gemini CLI): a tool with the same name, `SandboxNetworkAccess`, asks to run. Only Ned's is the sandbox's request. | Ned's conversation and Gil's, side by side. |
| `?approval=permissions` | Kit (Codex) asks for more permissions: to write in `../acme-docs`, outside its worktree. | Kit's conversation. |
| `?approval=question-long` | Ned (Claude Code) asks a question with two options of about 120 characters each. Kit (Codex) passes on a question of an MCP server that has no options: you write the answer. | Answer both (write a long answer for Kit), then look at the two closed rows. |

## The second office: `billing-api`

`acme/web` is not the only office of the demo. The tab `billing-api` says that two of its tasks
are in flight and that one request waits for you. Open it and the demo switches for real: it has
its own crew of three (Otto, Rex and Pia), one request with a plan that runs, and Otto's request
before `docker compose up -d stripe-mock`. Answer it and the plan finishes. Go back to `acme/web`
and it is as you left it, plus what its agents did meanwhile: both offices keep working, and a
request that waits in the one you are not looking at shows as a notice. A draft you typed in a
request of one office is there again when you come back to it.

The wallet is one for both offices: XP and coins are the same whichever office is open.

## What the demo does as the app does

- A message you send while a Claude Code or Codex agent is in a turn is read after the step the
  agent is in, and gets no turn of its own. With another CLI it waits for the turn's end. The
  wait is about a second: slow the script down (`?speed=0.25`) to see the "Queued" line.
- "Always allow in this repo": Ada asks before `npm install -D @testing-library/react`. Save the
  rule for `npm install` on that card, and Cy's later `npm install -D @axe-core/playwright` is
  allowed without a question.
  A command that deletes (Bo's `rm -rf dist && npm run build`) offers no rule.
- A 1:1 Claude agent you hire works in its mode: "plan first" shows a plan, "ask for edits" asks
  before the edit, "accept edits" edits without a question.
- Close office (on the map) refuses while the office has unfinished work or an agent is in a turn.
  In the demo, `billing-api` has agents at work, and `acme/web` closes once its open tasks are
  done or cancelled.
- MCP servers: the first Lead of an office (a Claude Code one) reports three when its session
  starts: `codly` is connected, `linear` needs a sign-in, `postgres` did not start. Its sheet shows
  the section "MCP servers". Reconnect connects `postgres` after a second; `linear` still needs
  its sign-in, and the row says so. Stop the Lead and the section is gone with its process, as in
  the app; it is back when the Lead works again.
- A command an agent leaves running in the background: in her first task Ada starts a type check
  (`npx tsc --noEmit --watch`) and stops it before she reports. While it runs, her sheet lists it
  under "In the background" with Stop, and her History has one row for each change of the list.
  Without `?autoplay=1` it stays until you answer her `npm install` request and then her
  question about the `localStorage` fallback: run the first plan (`?skip=20`), open Ada's sheet,
  and press Stop. Stop Ada herself and the section is gone: the
  command ended with her process, as in the app.

## Other demo pages

`office-demo.html` (the office alone) and `character-lab.html` (the characters) read parameters
of their own: see `src/office/demo.ts`, `src/office/demo/Harness.svelte` and
`src/office/demo/lab.ts`. They are tools for the office's look, not part of the app's demo.

## What the demo cannot show

There is no git, no `gh` and no shell behind it. A diff, a commit, a pull request and the output of
a command are made up. A 1:1 agent's branch always merges without a conflict. Look at these in the
app.

`billing-api` has its own agents, requests, plan and diffs, but the demo has one made-up repo: the
code map, the file browser, the search and the terminal of `billing-api` show `acme/web`'s files,
and its Lead plans nothing new there.
