# Drivers for the Claude Code stream-json recordings

These scripts made the fixtures in `docs/research/fixtures/claude-2.1.311/` and the four documents
`docs/research/cli-claude-stream-json-2.1.311.md` (part 1, scripts `s*.py`),
`docs/research/cli-claude-stream-json-2.1.311-part2.md` (part 2, scripts `r4_*.py`),
`docs/research/cli-claude-stream-json-2.1.311-part3.md` (part 3, scripts `r5_*.py`) and
`docs/research/cli-claude-stream-json-2.1.311-part4.md` (part 4, scripts `r6_*.py`).
Python 3.11 or newer, standard library only. They start the real `claude` CLI with your own login: read
"Before you run" first.

## Before you run

- Every run spends tokens of the logged-in account. All scripts pass `--model haiku` and use tiny prompts. At list
  price (sum of `total_cost_usd` of the `result` lines) part 1 cost 0.12 USD and part 2 cost 0.18 USD (0.19 USD with
  the runs that were recorded again); 0.11 USD of part 2 is one plan-mode turn that the CLI ran on sonnet by itself.
  Part 3 cost 0.18 USD (about 0.23 USD with the runs that were recorded again); 0.08 USD of it is
  `r5_c8_model_switch.py midturn`, the one script that leaves haiku on purpose (two model requests on sonnet, one on
  the account's default model), and 0.03 USD are two one-turn starts on `claude-haiku-4-5`.
  `cost.py` sums your recordings (a session that is resumed in several recordings is counted once per recording,
  with its running total: for part 3 it prints 0.18).
- The driver removes every `CLAUDE*` and `ANTHROPIC*` variable from the CLI's environment (a CLI started from
  inside a Claude Code session must not inherit that session's variables). It never reads or prints credentials.
- No script uses `bypassPermissions`. Each run works in a throw-away git repo under `$CAPTURE_DIR/work`.
- All runs pass `--strict-mcp-config --setting-sources local`, so your own skills, plugins, hooks and MCP servers stay
  out. One exception, on purpose: `r4_e19_project_mcp.py project` runs without `--strict-mcp-config`; the CLI then
  also connects your own MCP servers. That script sends no `mcp_status`, and the sanitizer drops every server that
  is not the scenario's own. Part 3 has two more of this kind: `r5_e11_mcp_sources.py clash` and
  `r5_e12_project_mcp_user_flags.py login-flag-disabled`. Both turn the claude.ai connectors off for the run (the
  `disableClaudeAiConnectors` setting and `ENABLE_CLAUDEAI_MCP_SERVERS=false`); no server of the user's own setup
  was in their init lines.
- Throw-away HOME: `r5_e11_mcp_sources.py user` and `r5_e12_project_mcp_user_flags.py` (all variants but
  `login-flag-disabled`) start the CLI with `HOME` set to a new folder under `$CAPTURE_DIR/work`, so that "user
  settings" are the scenario's own files and never yours. Such a CLI is not logged in: its turns end with "Not
  logged in" and cost nothing. No script copies or reads credentials to get around that.
- `r4_b7_process_groups.py`, `r4_b6_background_life.py` and the part 3 scripts that start a long `sleep` or a tick
  script through the CLI (`r5_a4`, `r5_b1`, `r5_f15`) kill what they started. They find it by its command line
  (`sleep 113`, `sleep 307`, `sleep 61`, `tick_<run>.sh`: values chosen so that nothing else on the machine has them)
  and never touch a process that existed before the run. If you add a script, keep to that: a pattern like
  `sleep 20` would also end another program's `sleep 20` that started during the run. `r5_b2` kills nothing: its
  commands end by themselves within 20 s.
- `r4_e18_mcp_needs_auth.py` listens on `127.0.0.1:48731` (`FAKE_MCP_PORT` changes it) for the time of the run.
- Sandbox scripts (`s09`, `r4_d14` … `r4_d17`, `r5_c13`, `r5_d6`): the CLI's sandbox lets commands write under its
  own temp folder (`/tmp/claude-<uid>/`). If `CAPTURE_DIR` is inside that folder, "outside the work dir" probes prove
  nothing (part 2, scenario 17). Put `CAPTURE_DIR` somewhere else for those (part 3 was recorded under the system's
  per-user temp folder, `$(getconf DARWIN_USER_TEMP_DIR)`).
- `r5_c7_effort_started_high.py` makes the CLI write a debug log to `$CAPTURE_DIR/raw/<scenario>.debug.log`. It has
  paths of your machine: never commit it.

Part 4 (`r6_*.py`), what is new:

- Cost at list price: 0.13 USD for the committed recordings (about 0.21 USD with the runs that were recorded again).
  0.05 USD of it is `r6_c5_model_while_streaming.py`, the one script that leaves haiku on purpose (one tiny turn on
  sonnet per recording), and 0.05 USD are the five runs of `r6_e8` that load your own setup (a longer first request).
  `cost.py` prints 0.15 for part 4: it counts a session once per recording that resumes it (sections 9 and 10).
- FIVE RUNS READ YOUR OWN USER SETTINGS with your real login: `r6_e8_project_mcp_login.py` with `enable-all-false`,
  `enabled-other`, `disabled`, `user-only` or `user-local-file` (`--setting-sources user…`, no `--strict-mcp-config`:
  that is the question they answer). So that they change nothing of yours, each adds `"disableAllHooks":true` to
  `--settings` (your hooks do not run), turns the claude.ai connectors off, allows one tool
  (`mcp__ping-project__ping`) and denies every other request, and sends no `mcp_status`. Run `hooks-on` and
  `hooks-off` first: they check, with a throw-away HOME, that the hook switch works on your CLI version. Your own
  stdio MCP servers are still started by the CLI, as in any session of yours. The sanitizer removes them from the
  fixtures and replaces the name of your output style; read the five fixtures before you commit them.
- Throw-away HOME (no login, no cost): `r6_e8_project_mcp_login.py` (`user-only-home`, `user-local-home`,
  `user-local-file-home`, `hooks-on`, `hooks-off`) and `r6_e11_mcp_clash_user.py`.
- The `user-local-file` variants of `r6_e8` write a `.claude/settings.local.json` into the scenario's WORK FOLDER (it
  turns the project's servers on, and has two harmless hooks that show the CLI read it). No file of yours.
- Processes: `r6_a1`, `r6_a2` and `r6_a3` start a long `sleep` through the CLI (`sleep 127`, `43`, `59`, `139`, `67`,
  `131`) and kill what is left of it, found by its command line as before. `r6_a3_helper_fails.py signal-*` sends ONE
  signal to a process of the scenario's own background command: the shell that the CLI started for it (a child of the
  CLI whose command line has the command) or that shell's `sleep` child; it stops without a signal when it does not
  find exactly one of each. `r6_e7_mcp_live_server.py` kills the scenario's MCP server, by the pid the server wrote,
  only when that pid's command line is this server's.
- Files outside the capture folder: `r6_f9_resume_elsewhere_damaged.py` writes six cut copies of ITS OWN session's
  transcript into that session's folder under `~/.claude/projects/` (under new session ids) and deletes them at the
  end. It changes no other file there. The CLI resumes them, so the copies do get lines appended before they go.
- Sandbox: `r6_d6_network_deny.py` (keep `CAPTURE_DIR` outside the CLI's temp folder, as for part 3).

## Layout

`CAPTURE_DIR` (default: the folder that holds this folder) gets three sub-folders:

- `raw/<scenario>.ndjson`: unsanitized recording, one object per line:
  `{"t": seconds since spawn, "dir": "recv"|"send"|"stderr"|"meta", "msg": …}`. Never commit these.
- `fixtures/<scenario>.ndjson`: sanitized, the format described in part 1 ("Files").
- `work/<scenario>/`: the throw-away repos.

Other variables: `CLAUDE_BIN` (default: `claude` on PATH), `FIXTURES_DIR` (where `sanitize.py` writes and the notes
builders read; default `$CAPTURE_DIR/fixtures`), `CLI_VERSION` and `DOC_NAME` (text of each fixture's first line).

## Record again

```sh
export CAPTURE_DIR=/some/empty/folder        # not inside the repo, not inside /tmp/claude-<uid>/
claude --version                              # name the fixtures folder after what this prints
cd docs/research/drivers/claude-stream-json

python3 r4_a1_queue_interrupt.py plain        # one scenario; variants are the first argument (see each docstring)
python3 tl.py r4-queue-interrupt-plain        # timeline of the raw recording (tlq.py: shorter)

python3 sanitize.py                           # raw/*.ndjson -> fixtures/*.ndjson
python3 make_notes_part2.py out.md            # part 2 document from the r4-* fixtures
python3 make_notes_part3.py out.md            # part 3 document from the r5-* fixtures (it checks its own statements)
python3 make_notes_part4.py out.md            # part 4 document from the r6-* fixtures (the same)
python3 cost.py                               # list-price cost of everything in raw/
```

Part 1 again, with the tools as they were used for the committed fixtures:

```sh
for s in s00_smoke s01_mid_turn …; do python3 $s.py; done      # see the variants in each docstring
LEGACY_TIME_MARKS=1 DOC_NAME=NOTES.md python3 sanitize.py       # part 1's time-marker rule
python3 make_notes.py out.md
```

(The recordings themselves differ from run to run: model text, ids, timings. Checked for the tools: from the raw
files of part 1, `LEGACY_TIME_MARKS=1 sanitize.py` gives the committed part 1 fixtures byte for byte. The committed
part 1 document is the output of `make_notes.py` plus a few hand edits: the "Files" list and two added notes.)

After sanitizing, check before anything goes into the repo:

```sh
cd "$CAPTURE_DIR/fixtures"
/usr/bin/grep -l -i "$USER" *.ndjson; /usr/bin/grep -l "$HOME" *.ndjson
/usr/bin/grep -o -h -E '[A-Za-z0-9._+-]+@[A-Za-z0-9-]+\.[A-Za-z]{2,}' *.ndjson | sort -u      # only demo@example.invalid
/usr/bin/grep -o -h -E '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' *.ndjson | sort -u
/usr/bin/grep -o -h 'mcp__[A-Za-z0-9_-]*' *.ndjson | sort -u                                   # only scenario servers
```

## Files

| file | what it is |
|---|---|
| `drv.py` | the driver: starts the CLI, logs both directions, answers `can_use_tool`, sends control requests |
| `sanitize.py` | raw -> fixture: paths, host name, ids, account fields, machine lists and hooks, big images, foreign MCP servers |
| `test_sanitize.py` | tests of the sanitizer rules that keep the machine out of a fixture (`python3 -m unittest test_sanitize`) |
| `tl.py`, `tlq.py` | timeline viewers for a raw recording |
| `cost.py` | sum of `total_cost_usd` over the recordings |
| `notes_lib.py`, `make_notes_part2.py` | build the part 2 document; every quoted line is copied from a fixture |
| `make_notes_part3.py` | builds the part 3 document the same way, and fails when a number or word its text states is not what the fixtures say |
| `make_notes_part4.py` | builds the part 4 document, with the same checks (the helpers both builders share are in `notes_lib.py`) |
| `make_notes.py` | builds the part 1 document |
| `bgrep.py` | finds a text in the CLI binary (for statements marked "bundle, not recorded") |
| `mcp_ping_server.py` | minimal stdio MCP server used by `r4_e19_project_mcp.py` and the part 3 MCP scripts (options: a label in the answer, a slow tool, a file it needs to start) |
| `r5_lib.py` | helpers shared by the `r5_*.py` scripts (the `r6_*.py` scripts use them too) |
| `r6_lib.py` | helpers of the `r6_*.py` scripts: time stamps of lines (not of the driver's waits), a note at the moment the process exits, process helpers |

Part 1 scenarios (`s*.py` -> fixtures without prefix): `s00_smoke`, `s01_mid_turn` (plain, uuid, uuid-now,
uuid-next, uuid-later), `s02_control`, `s03_image`, `s04_subagent`, `s05_background`, `s06_limits`,
`s07_resume_missing`, `s07b_session_id_in_use`, `s08_broken_mcp`, `s09_sandbox` (allow, deny-unsandboxed, strict,
off), `s10_permissions` (default, plan, ask), `s10b_updated_permissions`, `s11_slash`, `s12_misc`.

Part 2 scenarios (`r4_*.py` -> fixtures `r4-*`):

| script | variants | fixtures | part 2 scenario |
|---|---|---|---|
| `r4_a1_queue_interrupt.py` | plain, cancel-queued | `r4-queue-interrupt-*` | 1 |
| `r4_a2_two_queued.py` | tools, notool | `r4-two-queued-*` | 2 |
| `r4_a3_slash_mid_turn.py` | context, compact | `r4-slash-mid-turn-*` | 3 |
| `r4_a4_background_and_queue.py` | fold, later | `r4-background-and-queue-*` | 4 |
| `r4_a5_cancel_async.py` | | `r4-cancel-async-message` | 5 |
| `r4_b6_background_life.py` | | `r4-background-life` | 6 |
| `r4_b7_process_groups.py` | blocked, stdin, sigterm, sigkill | `r4-process-groups-*` | 7 |
| `r4_b8_tool_progress.py`, `r4_b8b_tool_progress_output.py` | | `r4-tool-progress`, `r4-tool-progress-output` | 8 |
| `r4_c9_accept_edits.py` | | `r4-accept-edits` | 9 |
| `r4_c10_plan_from_default.py`, `r4_c10_unknown_tools.py` | | `r4-plan-from-default`, `r4-unknown-tool-names` | 10, 11 |
| `r4_c12_compact_empty.py` | | `r4-compact-empty` | 12 |
| `r4_c13_model_effort_fallback.py` | | `r4-model-effort-fallback` | 13 |
| `r4_d14_sandbox_network.py` | allow, deny | `r4-sandbox-network-*` | 14 |
| `r4_d15_sandbox_modes.py` | noauto, manual, manual-noauto | `r4-sandbox-noauto`, `r4-sandbox-manual`, `r4-sandbox-manual-noauto` | 15 |
| `r4_d16_sandbox_required.py` | available, unavailable (optional: not committed) | `r4-sandbox-required-*` | 16 |
| `r4_d17_sandbox_worktree.py` | linked, plain, linked-off | `r4-sandbox-worktree-*` | 17 |
| `r4_e18_mcp_needs_auth.py` | | `r4-mcp-needs-auth` | 18 |
| `r4_e19_project_mcp.py` | project, strict | `r4-project-mcp-*` | 19 |
| `r4_f20_big_image.py` | | `r4-big-image` | 20 |

Part 3 scenarios (`r5_*.py` -> fixtures `r5-*`; the number in a script's name is the section of part 3):

| script | variants | fixtures | part 3 section |
|---|---|---|---|
| `r5_b1_stop_subagent.py` | 1, 2 (a label: the same scenario twice) | `r5-stop-subagent-*` | 1 |
| `r5_b2_background_fail.py` | | `r5-background-fail` | 2 |
| `r5_b3_tool_progress_other.py` | mcp, subagent, webfetch | `r5-tool-progress-*` | 3 |
| `r5_a4_stop_sequence.py` | sequence, three-plain, three-cancel, folded-plain, folded-cancel | `r5-stop-*` | 4 |
| `r5_c13_commands_that_ask.py` | off, noauto, manual-off, manual-noauto | `r5-commands-*` | 5, 13 |
| `r5_d6_sandbox_network_again.py` | once, once-later (waits 300 s: about 6 minutes), once-restart (two processes: fixtures `-a`, `-b`), rule-local, rule-session | `r5-sandbox-network-*` | 6 |
| `r5_c7_effort_started_high.py` | high, none | `r5-effort-started-*` | 7 |
| `r5_c8_model_switch.py` | midturn, start-alias, start-full, start-dated, start-undated | `r5-model-*` | 8 |
| `r5_f9_resume_missing.py` | (five runs in one call) | `r5-resume-missing-1` … `-5` | 9 |
| `r5_e10_mcp_reconnect_toggle.py` | | `r5-mcp-reconnect-toggle` | 10 |
| `r5_e11_mcp_sources.py` | config, clash, user | `r5-mcp-source-*` | 11 |
| `r5_e12_project_mcp_user_flags.py` | plain, flag-disabled, flag-enable-all-false, flag-enabled-other, user-disabled, login-flag-disabled | `r5-project-mcp-*` | 12 |
| `r5_f14_budget_resume.py` | (four runs in one call) | `r5-budget-resume-a` … `-d` | 14 |
| `r5_f15_background_wording.py` | rule-1 … rule-3, plain-1 … plain-3 | `r5-background-wording-*` | 15 |

Part 4 scenarios (`r6_*.py` -> fixtures `r6-*`; the number in a script's name is the section of part 4):

| script | variants | fixtures | part 4 section |
|---|---|---|---|
| `r6_a1_interrupt_helper.py` | foreground, background, idle; then a label (1, 2) | `r6-interrupt-helper-*` | 1 |
| `r6_a2_shutdown_helper.py` | sequence, idle-interrupt, idle-close-now, idle-interrupt-again, idle-outwait, foreground-stop; then a label (1, 2) | `r6-shutdown-helper-*` | 2 |
| `r6_a3_helper_fails.py` | helper-background, helper-foreground, signal-term, signal-kill, signal-child-kill | `r6-fails-*` | 3 |
| `r6_c4_effort_null_first.py` | fresh, after-turn | `r6-effort-null-first-*` | 4 |
| `r6_c5_model_while_streaming.py` | a label (1, 2) | `r6-model-while-streaming-*` | 5 |
| `r6_d6_network_deny.py` | a label (1, 2) | `r6-sandbox-network-deny-*` | 6 |
| `r6_e7_mcp_live_server.py` | status-first, call-first | `r6-mcp-live-server-*` | 7 |
| `r6_e8_project_mcp_login.py` | enable-all-false, enabled-other, disabled, user-only, user-local-file (real login, your user settings: see "Before you run"), user-only-home, user-local-home, user-local-file-home (then a label: 1, 2), hooks-on, hooks-off | `r6-project-mcp-*` | 8 |
| `r6_f9_resume_elsewhere_damaged.py` | (ten runs in one call) | `r6-resume-source`, `r6-resume-other-folder-1` … `-3`, `r6-resume-damaged-tail-1` … `-3`, `r6-resume-damaged-mid-1` … `-3` | 9 |
| `r6_f10_budget_midturn_resumed.py` | a label (1, 2) | `r6-budget-midturn-<label>-a`, `-b`, `-c1` … | 10 |
| `r6_e11_mcp_clash_user.py` | clash, strict; then a label (1, 2) | `r6-mcp-user-*` | 11 |

Notes for whoever records again:
- The `tool_use` line of a Bash call can come seconds before the call's process exists (the first shell start of a
  session is slow, more so on a busy machine). A scenario that stops a RUNNING command must wait for the process
  (`r6_lib.wait_alive`), not for the line: a first set of part 4 recordings stopped calls that had not started yet.
- Measure times with the stamps of the lines (`r6_lib.stamps`), never around `wait_results()`: that wait looks every
  0.2 s, and part 3 once reported that interval as a latency.
- `r6_f10_budget_midturn_resumed.py` tries budgets of 0.5, 0.7 and 0.85 of one turn's cost and stops at the first
  run whose limit came after a tool call had run: how many `-c` fixtures a label has differs from run to run. In a
  try with a very small budget the model may refuse to call a tool at all.
- The model does not always do what the prompt says (it adds an `ls`, it refuses a retry). Read the timeline before
  you trust a recording, and record again when the run did not reach the point of the scenario.
- A Bash command that starts with `sleep N` (N >= 25) is refused by the CLI itself. Keep waits short or start the
  command with something else.
- `r4_c10_plan_from_default.py` writes a plan file under `~/.claude/plans/`; every run leaves a session transcript
  under `~/.claude/projects/<work dir name>/`. Delete them yourself if you want them gone.
- The model skips a step it thinks is pointless: it did not call the Write tool for content the file already had,
  and not after a shell write to the same file had been blocked (`r5_c13` asks for new content, tool first).
- `r5_f14_budget_resume.py` stops when its budget does not fit between one turn and the session's earlier cost
  (the cache makes turn costs move): run it again.
- `r5_c13_commands_that_ask.py` sets up its work repo once (a bare repo inside it, a first push). Delete
  `$CAPTURE_DIR/work/r5-commands-*` before you record it again, or the commit and the push have nothing to do.
