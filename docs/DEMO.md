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
| `?skip=90` | Starts 90 seconds into the script. It also opens the repo. |
| `?autoplay=1` | Runs each plan and allows each request by itself. |
| `?onboard=1` | A machine with no agent CLI installed, to see the first-run quest. |

`?autoplay=1&skip=180` is the short way to both plans finished.

## States the script does not reach

Each of these opens `acme/web` at once, except `?trust=ask`.

| Parameter | What you get | Where to look |
| --- | --- | --- |
| `?trust=ask` | The repo has CLI settings of its own, and you were never asked about them. | Open `acme/web`: the trust dialog. |
| `?merge=conflict` | Your branch changed a file that each finished plan changed too. Merge is refused. "Have an agent resolve it" gives the plan one more task; when it is done, the plan merges. | The plan's finished card: `?merge=conflict&autoplay=1&skip=180` |
| `?preview=running` | A command you started earlier runs in each worktree you look at. | "Try it before you merge" on a finished plan's card (`?preview=running&autoplay=1&skip=180`), or on the Git tab of a 1:1 agent. |
| `?intake=issue`, `?intake=pr` | The "From issue…" (or "From pull request…") dialog is open, and the scripted requests do not start: yours is the first. Any number works. Run the plan, then "Create pull request…": the description ends with `Closes #N`. | The dialog, then the request it opens. |
| `?helper=slow` | The helper (sub-agent) that Ada starts in her first task works for about a minute, not three seconds. | Ada's work in the first request: `?helper=slow&autoplay=1&skip=20` |
| `?checks=on` | The repo's required checks are on, so Codly runs them for each task after the task's own checks. | A finished task's sheet, under its checks ("required by you"): `?checks=on&autoplay=1&skip=180` |
| `?mode=plan` | Eli, a 1:1 Claude agent in the mode "plan first", was asked for a small change and shows a plan. Send it back with a note and it comes again; accept it and Eli asks before the edit. | Eli's conversation; the mode switch is in Eli's sheet, under Controls. |

## What the demo does as the app does

- A message you send while a Claude Code or Codex agent is in a turn is read after the step the
  agent is in, and gets no turn of its own. With another CLI it waits for the turn's end. The
  wait is about a second: slow the script down (`?speed=0.25`) to see the "Queued" line.
- "Always allow in this repo": Ada asks before `npm install -D @testing-library/react`. Save the
  rule for `npm install` on that card, and Cy's later `npm install` is allowed without a question.
  A command that deletes (`rm -rf dist && npm run build`) offers no rule.
- A 1:1 Claude agent you hire works in its mode: "plan first" shows a plan, "ask for edits" asks
  before the edit, "accept edits" edits without a question.
- Close office (on the map) refuses while the office has unfinished work or an agent is in a turn.
  In the demo, `billing-api` has agents at work, `docs-site` closes, and `acme/web` closes once its
  open tasks are done or cancelled.
- A command an agent leaves running in the background: in her first task Ada starts a type check
  (`npx tsc --noEmit --watch`) and stops it before she reports. While it runs, her sheet lists it
  under "In the background" with Stop, and her History has one row for each change of the list.
  Without `?autoplay=1` it stays until you answer her `npm install` request: run the first plan
  (`?skip=20`), open Ada's sheet, and press Stop.

## What the demo cannot show

There is no git, no `gh` and no shell behind it. A diff, a commit, a pull request and the output of
a command are made up. A 1:1 agent's branch always merges without a conflict. Look at these in the
app.
