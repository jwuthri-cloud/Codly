# Claude Code CLI stream-json: recorded protocol shapes, part 5

- CLI: `claude` 2.1.311 (the same version as parts 1 to 4; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 to 4. Fixtures of this part are `fixtures/claude-2.1.311/r7-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r7_*.py` (README there says how to record again).
- List price of the 43 recordings (sum of `total_cost_usd` of their `result` lines, per session): 0.0500 USD. The runs
  in a throw-away HOME have no login and cost nothing.
- Everything below was observed in these recordings. Each answer says how many runs it rests on (n) and says no more
  than the recordings show.
- The sections are numbered as the 12 questions this part was recorded for; the last two tables have them in the same
  order.

Differences from the part 4 method:
- Times are time stamps of the recording, as in part 4. A note that lists lines is now written under the driver's lock
  in the same step as the list, and the process checks are made before that step (`r7_a5`, and `r6_a2` fixed the same
  way): no line can come between a list and its note's stamp. Part 4 had notes written late: see "Notes of part 4
  written late".
- EVERY RUN THAT SWITCHES AN MCP SERVER uses a throw-away HOME: section 1 shows that Claude Code saves the switch in
  `~/.claude.json`. (Earlier parts switched servers with the real HOME: `r4-mcp-needs-auth`, `r5-mcp-reconnect-toggle`.
  Both turned their server on again before they ended, which empties the saved list again: section 1.)
- Two recordings of section 2 run with the REAL login and the user's own settings (`--setting-sources user…`, no
  `--strict-mcp-config`), because that is the question. Same safety as part 4: `"disableAllHooks":true` in
  `--settings`, the claude.ai connectors off, one tool allowed and every other request denied, no `mcp_status`; the
  sanitizer removes the user's own servers and names the output style with a placeholder.
- Section 11 also counts, from the session's own transcript file that Claude Code writes under `~/.claude/projects/`,
  the task notifications that waited in its queue and those it handed to a running turn (`r7_lib.queued_notifications`:
  counts only, the transcript itself is never copied).
- Section 10 splits a run's cost into its model requests: each request of the main thread at the list prices of
  `claude-haiku-5-5` from its final usage (the `message_delta` stream event), the helper's share as the rest of
  `total_cost_usd` (the CLI streams no events of a helper). The prices are the ones that give the `costUSD` of every
  `model_usage_since_last_result` of these recordings to the last digit.
- New sanitizing: the MCP servers `ping-http` and `ping-local` are kept in `init`, `mcp_status` and `can_use_tool`.
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started. A quoted
  line of a subagent keeps `parent_tool_use_id` and `agent_id`.

## 1. `mcp_toggle` off, the process exits, a NEW process in the same folder

Question: what does `mcp_status` say about that server in a new process (fresh, and with `--resume`)? Is the switch saved, and where?

Fixtures: `r7-mcp-toggle-saved-1-a`, `r7-mcp-toggle-saved-1-b`, `r7-mcp-toggle-saved-1-c` (label 1), `r7-mcp-toggle-saved-2-a`, `r7-mcp-toggle-saved-2-b`, `r7-mcp-toggle-saved-2-c` (label 2). A THROW-AWAY HOME (no login: the one message of each process ends at once with `Not logged in`). Each process: `ping-live` with `--mcp-config` (one line in `started.txt` per start), flags `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r7-mcp-toggle-saved-1.json`. Process a: `mcp_status`, `mcp_toggle` off, `mcp_status`, one message, stdin closed. Process b: a NEW session in the same folder: `mcp_status`, one message. Process c: `--resume` of a's session in the same folder: `mcp_status`, then `mcp_toggle` on, `mcp_status`, one message. After each step a note names the files under the throw-away HOME and the work folder that changed, and the folder's entry in `HOME/.claude.json` (its MCP keys).

Process a, the switch and the note after it:

```json
{"type":"control_request","request_id":"host-2","request":{"subtype":"mcp_toggle","serverName":"ping-live","enabled":false}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2"}}
{"after":"mcp_toggle off","files_changed":["r7-mcp-toggle-saved-1-home/.claude.json"],"project_entry_mcp_keys":{"mcpContextUris":[],"mcpServers":{},"enabledMcpjsonServers":[],"disabledMcpjsonServers":[],"disabledMcpServers":["ping-live"]},"server_starts_so_far":1}
{"type":"control_request","request_id":"host-3","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"mcpServers":[{"name":"ping-live","status":"disabled","config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic"}]}}}
```

Process b (a new session), `mcp_status` and the `init` line of its one message:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"mcpServers":[{"name":"ping-live","status":"disabled","config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic"}]}}}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[{"name":"ping-live","status":"disabled","source":"dynamic"}],"…":"…"}
```

Process c (`--resume` of a's session): `mcp_status`, the switch back on, the note after it:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"mcpServers":[{"name":"ping-live","status":"disabled","config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic"}]}}}
{"type":"control_request","request_id":"host-2","request":{"subtype":"mcp_toggle","serverName":"ping-live","enabled":true}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2"}}
{"after":"mcp_toggle on","files_changed":["r7-mcp-toggle-saved-1-home/.claude.json","r7-mcp-toggle-saved-1/started.txt"],"project_entry_mcp_keys":{"mcpContextUris":[],"mcpServers":{},"enabledMcpjsonServers":[],"disabledMcpjsonServers":[],"disabledMcpServers":[]},"server_starts_so_far":2}
```

What the adapter can rely on (n=2, the two labels agree step for step):
- THE SWITCH IS SAVED. `mcp_toggle` off writes ONE file: `HOME/.claude.json` (the throw-away HOME's; for a real run,
  the user's own `~/.claude.json`). The folder's entry there (under `projects`, keyed by the folder's path) gets
  `"disabledMcpServers":["ping-live"]`. The entry's MCP keys: `disabledMcpServers`, `disabledMcpjsonServers`, `enabledMcpjsonServers`, `mcpContextUris`, `mcpServers`.
- It holds for EVERY later process in that folder that reads that HOME: a new session (b) and a resumed one (c) both
  report the server `disabled` in `mcp_status` and in `init.mcp_servers` (`"source":"dynamic"`), its tool is not in
  `init.tools`, and the server is NOT started (no new line in `started.txt`). The saved list is read with
  `--strict-mcp-config --setting-sources local` (all six processes; other setting sources were not tried).
- The switch goes by the server's NAME in that folder: the server came from `--mcp-config` (Codly's way), so any
  server of that name in that folder stays off, also one that Codly passes in a later run.
- Only `mcp_toggle` on undoes it: the list is `[]` again in the same step, and the server starts (`connected`).
- So "Turn off" in the app is not "for this run": it is for this folder, for every later Claude Code run there under
  the same HOME. Expected, not recorded: the user's own `claude` sessions in that folder read the same entry (for a
  server of the same name), and a worktree, being another folder, has its own entry.

## 2. Your own MCP servers in a folder you have not trusted, a second time

Question: without `--strict-mcp-config`, with `--setting-sources user` or `user,local`: is the repo's `.mcp.json` server loaded? Also with a local-scope server of the folder and with an approval of the project's servers kept in `~/.claude.json`.

Fixtures: `r7-untrusted-mcp-<variant>-<label>`. The work folder's `.mcp.json` defines `ping-project` (one line in `started.txt` per start). No run passes `--strict-mcp-config`. The two `-login` variants use the REAL login and your own user settings (safety as described above, as in part 4); the others a THROW-AWAY HOME (no login: they show what is LOADED). Flags of `user-login`: `--permission-mode manual --setting-sources user --tools Bash --settings '{"disableAllHooks": true, "disableClaudeAiConnectors": true}'`.

- `local-scope-*`: in the throw-away HOME, `claude mcp add --scope local ping-local -- python3 mcp_ping_server.py started-local.txt label=local` was run in the folder first (the CLI's own command writes the server into `HOME/.claude.json` under the folder's path).
- `approval-*`: the CLI first wrote the folder's entry (`claude mcp add --scope local`, then `remove`: the entry stays), and the script set two of its keys: `"enabledMcpjsonServers":["ping-project"]`, `"hasTrustDialogAccepted":true` (an approval of the repo's server kept there). It added no key.

The set-up note and the `init` line of `local-scope-user-local-1`:

```json
{"claude_mcp_add":{"exit_code":0,"first_line_of_its_output":"Added stdio MCP server ping-local with command: python3 mcp_ping_server.py started-local.txt label=local to local config"},"servers_started_by_the_setup_command":{"project":0,"local":0},"project_entry_in_home_claude_json":{"mcpServers":{"ping-local":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started-local.txt","label=local"],"env":{}}},"enabledMcpjsonServers":[],"disabledMcpjsonServers":[],"hasTrustDialogAccepted":false}}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","mcp__ping-local__ping"],"mcp_servers":[{"name":"ping-local","status":"connected","source":"local"}],"…":"…"}
```

The set-up note of `approval-user-1`:

```json
{"claude_mcp_remove_exit_code":0,"servers_started_by_the_setup_command":{"project":0,"local":0},"project_entry_in_home_claude_json":{"mcpServers":{},"enabledMcpjsonServers":["ping-project"],"disabledMcpjsonServers":[],"hasTrustDialogAccepted":true}}
```

| recording | login | `--setting-sources` | what the folder or HOME has | `ping-project` started | `ping-local` started | scenario servers in `init` | your own servers in `init` |
|---|---|---|---|---|---|---|---|
| `r7-untrusted-mcp-user-login-1` | real | `user` | `.mcp.json` only | 0 | 0 | none | yes |
| `r7-untrusted-mcp-user-local-login-1` | real | `user,local` | `.mcp.json`; `.claude/settings.local.json` turns the project's servers on | 0 | 0 | none | yes |
| `r7-untrusted-mcp-local-scope-user-1` | none (throw-away HOME) | `user` | `.mcp.json`; a local-scope `ping-local` in `HOME/.claude.json` | 0 | 0 | none | no |
| `r7-untrusted-mcp-local-scope-user-2` | none (throw-away HOME) | `user` | `.mcp.json`; a local-scope `ping-local` in `HOME/.claude.json` | 0 | 0 | none | no |
| `r7-untrusted-mcp-local-scope-user-local-1` | none (throw-away HOME) | `user,local` | the same | 0 | 1 | `ping-local` (local) | no |
| `r7-untrusted-mcp-local-scope-user-local-2` | none (throw-away HOME) | `user,local` | the same | 0 | 1 | `ping-local` (local) | no |
| `r7-untrusted-mcp-approval-user-1` | none (throw-away HOME) | `user` | `.mcp.json`; the approval in `HOME/.claude.json` | 0 | 0 | none | no |
| `r7-untrusted-mcp-approval-user-2` | none (throw-away HOME) | `user` | `.mcp.json`; the approval in `HOME/.claude.json` | 0 | 0 | none | no |
| `r7-untrusted-mcp-approval-user-local-1` | none (throw-away HOME) | `user,local` | the same | 0 | 0 | none | no |
| `r7-untrusted-mcp-approval-user-local-2` | none (throw-away HOME) | `user,local` | the same | 0 | 0 | none | no |

What the adapter can rely on:
- The repo's `.mcp.json` server was loaded in NONE of these 10 recordings (no start, no `init` entry, no tool), nor in
  the 6 recordings of the same kind in part 4 (`r6-project-mcp-user-only`, `r6-project-mcp-user-only-home`, `r6-project-mcp-user-local-home`, `r6-project-mcp-user-local-file`, `r6-project-mcp-user-local-file-home-1`, `r6-project-mcp-user-local-file-home-2`). Logged in: n=2 for `user` (part 4 `user-only`, here
  `user-login-1`) and n=2 for `user,local` with a local file that turns the project's servers on (part 4
  `user-local-file`, here `user-local-login-1`).
- An approval of the project's server kept in `~/.claude.json` under the folder's path does not load it either, with
  `user` or with `user,local` (n=2 each, no login). Whether these keys count when `project` is a source was not
  recorded (an untrusted start does not give that source).
- A LOCAL-SCOPE server (the user's own, added with `claude mcp add --scope local`, kept in `~/.claude.json` under the
  folder's path) is loaded only with `local` among the sources: with `user,local` it starts and its entry says
  `"source":"local"`; with `user` alone it is not loaded (n=2 each, no login). It is the user's own server, not the
  repo's: the repo cannot add it.
- With your own setup, your own servers are loaded (the two logged-in runs: yes; the sanitizer removed them).

## 3. A turn that FAILS by itself while a background helper works

Question: does the helper go on, what does the list say, what ends it?

Fixtures: `r7-failed-turn-helper-1`, `r7-failed-turn-helper-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent --max-turns 2`. Turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 41; echo end`) and ends within the limit (two model requests). Turn 2 asks for three Bash calls one after the other: its third model request passes `--max-turns 2`.

From turn 2's message on: the task and list lines, the turns' `init` and `result` lines, the driver's notes (first recording):

```json
# t=8.29
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `echo one`. After you see its output, run `echo two` in a second, separate tool call. After that, run `echo three` in a third, separate tool call. Then reply with the single word: done"},"parent_tool_use_id":null},"_t":8.287}
# t=8.29
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=9.55
{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"max_turns","errors":["Reached maximum number of turns (2)"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=9.99
{"_meta":{"right_after_the_failed_turn":{"result_subtype":"error_max_turns","terminal_reason":"max_turns","num_turns":3,"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run delayed echo command"}],"sleep_41_alive":true,"lines":["system/init","assistant","…(4 more)"]}},"_t":9.995}
# t=11.15
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","owned_by_subagent":true,"tool_use_id":"toolu-2","description":"Print begin, sleep 41 seconds, print end","is_backgrounded":false,"parent_task_id":"tid-1","task_type":"local_bash"}
# t=49.20
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"completed","output_file":"","summary":"Print begin, sleep 41 seconds, print end"}
# t=49.73
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795470952685}}
# t=49.73
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r7-failed-turn-helper-1/sess-1/tasks/tid-1.output","summary":"The command ran and printed:\n\n```\nbegin\nend\n```","usage":"…"}
# t=49.73
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=49.73
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=51.43
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"done","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":2,"…":"…"}
# t=52.64
{"_meta":{"after_it":{"helper_end":"completed","helper_summary":"The command ran and printed:\n\n```\nbegin\nend\n```","turn_by_itself":true,"its_results":[["success",{"kind":"task-notification","producer":"session-task","runId":"run-1"}]],"lines":["system/task_started","system/task_notification \"completed\"","…(11 more)"],"list":[],"sleep_41_alive":false}},"_t":52.638}
# t=52.64
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":52.638}
# t=53.39
{"_meta":{"exit_code":0},"_t":53.388}
```

What the adapter can rely on (n=2, the two recordings agree line for line):
- A turn that ends with an error by itself (here `error_max_turns`, `terminal_reason:"max_turns"`) does NOT end a
  background helper. No task line and no list line comes with the failed turn; right after its `result` the list still
  names the helper and its `sleep 41` runs.
- The helper ends later by itself, as without the failure: `task_updated` `completed`, `task_notification` `completed`
  with its report in `summary`, the list `[]`, and then the CLI starts a turn by itself (`result` with
  `origin.kind:"task-notification"`).
- So the end of a turn says nothing about background helpers, whatever its status: interrupted (part 4: the stream
  ends every helper itself), failed (here: they go on) or done. A host that ends helpers only at their own
  `task_notification` is right in all three; a host that ends them at a failed turn is wrong.
- Not recorded: a turn that fails by an API error or by the budget while a helper works (the budget: section 10, where
  the helper had already finished).

## 4. An `interrupt` with TWO helpers at work, and with a helper that waits for its model answer

Question: are all ended, with which lines? (Part 4 recorded one helper, always inside a Bash call.)

Fixtures: `r7-interrupt-two-helpers-1`, `r7-interrupt-two-helpers-2` (two background helpers, each in a Bash call: `echo a; sleep 181; echo end` and `echo b; sleep 191; echo end`), `r7-interrupt-helper-model-1`, `r7-interrupt-helper-model-2` (one background helper that writes a story with no tool). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. No turn of the main agent runs when the interrupt comes. The CLI streams no partial lines of a helper: the `model` variant sends the interrupt 1.5 s after the result of turn 1, and its note lists the block types of the helper's `assistant` lines so far.

Two helpers: every line from the driver's note before the interrupt to the note after it (first recording):

```json
# t=6.78
{"_meta":{"before_the_interrupt":{"when":"3 s after both sleep processes were seen","helpers":2,"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run echo a sleep 181 command"},{"task_id":"tid-2","run_id":"run-2","task_type":"local_agent","subagent_type":"general-purpose","description":"Run echo b sleep 191 command"}],"a_turn_runs":false,"assistant_lines_of_helpers":[["tool_use"],["tool_use"]],"sleep_181_alive":true,"sleep_191_alive":true}},"_t":6.782}
# t=6.78
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":6.782}
# t=6.78
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795472279015}}
# t=6.78
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-interrupt-two-helpers-1/sess-1/tasks/tid-1.output","summary":"Run echo a sleep 181 command"}
# t=6.78
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-2","run_id":"run-2","task_type":"local_agent","subagent_type":"general-purpose","description":"Run echo b sleep 191 command"}]}
# t=6.78
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795472279016}}
# t=6.78
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-interrupt-two-helpers-1/sess-1/tasks/tid-2.output","summary":"Run echo b sleep 191 command"}
# t=6.78
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=6.78
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=6.78
{"type":"system","subtype":"task_notification","task_id":"tid-4","run_id":"run-4","tool_use_id":"toolu-3","status":"stopped","output_file":"","summary":"Echo a, sleep 181 seconds, echo end"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-3"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=6.78
{"type":"system","subtype":"task_notification","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-4","status":"stopped","output_file":"","summary":"Echo b, sleep 191 seconds, then echo end"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-4"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-2","…":"…"}
# t=6.78
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-2","…":"…"}
# t=16.85
{"_meta":{"after_the_interrupt":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(11 more)"],"end_words_per_helper":[["killed","stopped"],["killed","stopped"]],"helpers_ended":2,"results_in_the_10s_after":0,"list":[],"sleep_181_alive":false,"sleep_191_alive":false},"seconds_from_the_interrupt_request":{"answer":0.004,"first_task_notification":0.003,"result":null,"next_init":null}},"_t":16.854}
```

A helper that waits for its model answer (first recording):

```json
# t=3.82
{"_meta":{"before_the_interrupt":{"when":"1.5 s after the result of turn 1","helpers":1,"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Write lighthouse keeper story"}],"a_turn_runs":false,"assistant_lines_of_helpers":[["thinking"]],"sleep_181_alive":false,"sleep_191_alive":false}},"_t":3.823}
# t=3.82
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":3.823}
# t=3.82
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795470825086}}
# t=3.82
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-interrupt-helper-model-1/sess-1/tasks/tid-1.output","summary":"Write lighthouse keeper story"}
# t=3.82
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=3.82
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=3.82
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=13.90
{"_meta":{"after_the_interrupt":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(3 more)"],"end_words_per_helper":[["killed","stopped"]],"helpers_ended":1,"results_in_the_10s_after":0,"list":[],"sleep_181_alive":false,"sleep_191_alive":false},"seconds_from_the_interrupt_request":{"answer":0.006,"first_task_notification":0.006,"result":null,"next_init":null}},"_t":13.904}
```

What the adapter can rely on (n=2 per variant):
- TWO helpers: the interrupt ends both. For each: `task_updated` (`patch.status:"killed"`), `task_notification`
  (`"status":"stopped"`) and a list line (the list goes from 2 to 1 to 0), all BEFORE the answer to `interrupt`. Then
  each helper's running Bash call (a task of its own when it had run 3 s or more: section 12) gets its own
  `task_notification` `stopped`, and each helper's two `user` lines (the rejected call,
  `[Request interrupted by user for tool use]`) follow.
- A helper that waits for its MODEL answer (no tool running) is ended the same way: `task_updated` `killed`,
  `task_notification` `stopped`, the list `[]`, the answer; then ONE `user` line of the helper:
  `[Request interrupted by user]` (not "for tool use"). Its answer is never written.
- In all four: no `result` (no turn ran), no turn by itself in the 10 s after, the list empty, no `sleep` left. From the
  request to the answer: 0.004, 0.010, 0.006, 0.002 s.
- With part 4 (one helper inside a Bash call, n=8): every helper that was at work when the interrupt came was ended
  by it, in 12 of 12 recordings. Still not recorded: a nested helper, a helper with a background command of its own.

## 5. A shutdown with a background helper AND a background command

Question: `interrupt`, `stop_task` for the command, stdin closed: the order of the lines, the exit time, anything left running?

Fixtures: `r7-shutdown-helper-and-command-1`, `r7-shutdown-helper-and-command-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. The CLI is the leader of its own process group, as the app starts it. Turn 1 starts a background command (`echo bg; sleep 197`) and a background helper (its Bash call: `echo begin; sleep 199; echo end`) and ends; the host waits until it sees both `sleep` processes, and 2 s more.

Every line from the driver's note before the stop (first recording):

```json
# t=4.99
{"_meta":{"before_the_stop":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run background sleep 199 command"},{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"Run background echo and sleep 197"}],"a_turn_runs":false,"command_task":"tid-2","helper_task":"tid-1","sleep_197_alive":true,"sleep_199_alive":true}},"_t":4.985}
# t=4.99
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":4.985}
# t=4.99
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795470870628}}
# t=4.99
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-shutdown-helper-and-command-1/sess-1/tasks/tid-1.output","summary":"Run background sleep 199 command"}
# t=4.99
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"Run background echo and sleep 197"}]}
# t=4.99
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=4.99
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"stop_task","task_id":"tid-2"}},"_t":4.993}
# t=4.99
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795470870633}}
# t=4.99
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-shutdown-helper-and-command-1/sess-1/tasks/tid-2.output","summary":"Run background echo and sleep 197"}
# t=4.99
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=4.99
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{}}}
# t=4.99
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-3"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-1","…":"…"}
# t=4.99
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-2","agent_id":"tid-1","…":"…"}
# t=5.57
{"_meta":{"before_stdin_is_closed":{"lines_since_the_interrupt":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(8 more)"],"list":[],"sleep_197_alive":false,"sleep_199_alive":false},"seconds_from_the_interrupt":{"answer":0.008,"helper_notification":0.008,"command_notification":0.011},"seconds_from_stop_task":{"answer":0.003,"command_notification":0.003}},"_t":5.57}
# t=5.57
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":5.571}
# t=6.27
{"_meta":{"process_exited":true},"_t":6.273}
# t=6.58
{"_meta":{"exit_code":0},"_t":6.578}
# t=7.68
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":[],"sleep_197_alive":false,"sleep_199_alive":false,"processes_left_in_the_group":[]}},"_t":7.681}
```

What the adapter can rely on (n=2, the two recordings have the same lines in the same order):
- `interrupt` with no turn running ends the HELPER: `task_updated` `killed`, `task_notification` `stopped`, a list line
  that still names the COMMAND (1 listed), then the answer. The command is not touched by the interrupt (part 3,
  section 4, already showed that).
- `stop_task` with the command's id then ends the COMMAND: `task_updated` `killed`, `task_notification` `stopped`, the
  list `[]`, the answer `{}`. After it come the helper's two `user` lines (its rejected Bash call).
- No turn starts by itself, no `result` is written. After stdin close the process was gone in 0.70 / 0.70 s, exit code 0
  (no turn had failed). After the exit no `sleep` of the scenario was alive and the CLI's process group was empty.
- So the shutdown order of part 4 holds with both kinds at once: `interrupt` (also with no turn running), then
  `stop_task` for each command of the list, then close stdin. A helper needs no `stop_task`.

## 6. `tool_progress` for a tool of an HTTP MCP server

Question: is there a heartbeat line every 30 s, as for a stdio server (part 3, section 3)?

Fixtures: `r7-tool-progress-http-70-1`, `r7-tool-progress-http-70-2` (the tool waits 70 s), `r7-tool-progress-http-45-1` (control: 45 s). The server `ping-http` runs in the driver (Streamable HTTP on 127.0.0.1, JSON answers, no event stream: GET gets 405) and comes with `--mcp-config` as type `http`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r7-tool-progress-http-70-1.json`.

The call, its request, every `tool_progress` line, the tool result (first recording of 70 s):

```json
# t=1.22
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"mcp__ping-http__ping","input":{}}],"…":"…"},"…":"…"}
# t=1.22
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"mcp__ping-http__ping","mcp_server":{"name":"ping-http","source":"dynamic"},"display_name":"Ping","input":{},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"mcp__ping-http__ping"}],"behavior":"allow","destination":"localSettings"}],"server_policy_ask_only":true,"tool_use_id":"toolu-1"}}
# t=31.27
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-0","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":30,"heartbeat":true}
# t=61.27
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-1","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":60,"heartbeat":true}
# t=61.27
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The operation timed out.","is_error":true,"tool_use_id":"toolu-1"}]},"tool_use_result":"Error: The operation timed out.","tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"user_temporary"}}],"…":"…"}
```

The control of 45 s: the `tool_progress` line and the result:

```json
# t=31.19
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-0","tool_name":"mcp__ping-http__ping","parent_tool_use_id":"toolu-1","elapsed_time_seconds":30,"heartbeat":true}
# t=46.20
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":[{"type":"text","text":"pong after 45 s"}]}]},"…":"…"}
```

What the server got (first recording of 70 s; times are seconds since the server started, a moment before the CLI):

```json
[{"at":0.663,"http":"POST","jsonrpc_method":"server/discover"},{"at":0.685,"http":"POST","jsonrpc_method":"initialize","answered_at":0.685},{"at":0.688,"http":"POST","jsonrpc_method":"notifications/initialized"},{"at":0.69,"http":"GET"},{"at":0.693,"http":"POST","jsonrpc_method":"tools/list","answered_at":0.693},{"at":1.268,"http":"POST","jsonrpc_method":"tools/call"}]
```

What the adapter can rely on:
- Yes: the same heartbeat line as for a stdio server (`tool_use_id` `<call id>-heartbeat-N`, `parent_tool_use_id` =
  the call id, `tool_name` = the tool, `"heartbeat":true`) at 30 s and 60 s (n=2), at 30 s for the 45 s call (n=1).
- NEW: an HTTP MCP tool call does not outlast 60 s. At 60.027, 60.017 s after its `tool_use` line (n=2) the call ended with
  the error tool result `The operation timed out.` (`tool_use_result` `"Error: The operation timed out."`), while
  the server was still working on it; the turn went on (the model got the error). The 45 s call returned its answer.
  The stdio tool of part 3 ran 100 s and returned. Recorded with a server that answers with a JSON body; a server that
  streams its answer (an event stream) is not recorded.
- The approval request of an HTTP server's tool has `"mcp_server":{"name":"ping-http","source":"dynamic"}`, as for a
  stdio server of `--mcp-config`.

## 7. An allow answer with the request's own `setMode` suggestion

Question: an allow that carries `{"type":"setMode","mode":"acceptEdits","destination":"session"}` in `updatedPermissions`: what does `get_settings` say after it, does the next Edit ask, does the stream say that the mode changed?

Fixtures: `r7-allow-set-mode-1`, `r7-allow-set-mode-2`. Flags: `--permission-mode default --strict-mcp-config --setting-sources local --tools Read,Edit`. Turn 1 asks for one Edit of `hello.txt`; the first Edit request is answered `allow` with `updatedPermissions` = the request's own `setMode` suggestion, copied unchanged. Turn 2 asks for a second Edit.

The request, the driver's answer, the status line after it (first recording):

```json
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Edit","display_name":"Edit","input":{"file_path":"/tmp/work/r7-allow-set-mode-1/hello.txt","old_string":"hello","new_string":"hello one","replace_all":false},"description":"hello.txt","permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],"tool_use_id":"toolu-2"}}
{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"allow","updatedInput":{"file_path":"/tmp/work/r7-allow-set-mode-1/hello.txt","old_string":"hello","new_string":"hello one","replace_all":false},"updatedPermissions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}]}}}
# t=3.22
{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits"}
```

The driver's notes (turns and `get_settings`) and the `init` line of turn 2:

```json
{"turn":"1","ok":true,"init_permissionMode":"default","requests":[["Edit","hello.txt",["setMode"]]],"status_lines_with_permissionMode":["acceptEdits"]}
{"turn":"2","ok":true,"init_permissionMode":"acceptEdits","requests":[],"status_lines_with_permissionMode":[]}
{"step":"after turn 1","effective_permissions":null,"applied":{"model":"claude-haiku-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true},"sources":[]}
{"step":"after turn 2","effective_permissions":null,"applied":{"model":"claude-haiku-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true},"sources":[]}
# t=3.88
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Edit","Read"],"mcp_servers":[],"…":"…"}
```

What the adapter can rely on (n=2):
- One answer does both: the Edit runs, and the session is in `acceptEdits` from then on.
- The stream says it at once: one line `{"type":"system","subtype":"status","status":null,"permissionMode":"acceptEdits"}`
  after the answer, and the next turn's `init` has `"permissionMode":"acceptEdits"`.
- The next Edit does NOT ask (no `can_use_tool` request in turn 2), and it ran (its tool result is no error).
- `get_settings` does not tell the mode: its answer has no permission mode at all (`effective` `{}`, `sources` `[]`,
  `applied` with model and effort only), before and after. Read the mode from the `status` line or the next `init`.
- The same as part 1 (scenario 10: a `setMode` item among all the suggestions of a Bash request switched the session);
  here it is the Edit request's own and only suggestion.

## 8. `effortLevel: null` on a process started with `--effort low` or `--effort max`

Question: what does `get_settings` say after it?

Fixtures: `r7-effort-null-low-1`, `r7-effort-null-low-2`, `r7-effort-null-max-1`, `r7-effort-null-max-2`. Flags of the first: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --effort low`. Order: `initialize`, `get_settings`, `apply_flag_settings {"effortLevel": null}`, `get_settings`, one turn, `get_settings`.

The request and its answer (first recording):

```json
{"type":"control_request","request_id":"host-3","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":null}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3"}}
```

| recording | `applied.effort` as started | after `null` | after the next turn |
|---|---|---|---|
| `r7-effort-null-low-1` | `low` | `medium` | `medium` |
| `r7-effort-null-low-2` | `low` | `medium` | `medium` |
| `r7-effort-null-max-1` | `max` | `medium` | `medium` |
| `r7-effort-null-max-2` | `max` | `medium` | `medium` |

What the adapter can rely on (n=2 per level):
- `null` gives `medium` at once, whatever level the process was started with: `low` -> `medium`, `max` -> `medium`
  (and `high` -> `medium` in part 4, section 4). It stays `medium` through the next turn. `medium` is also what a
  process started with no `--effort` reports (part 3, `r5-effort-started-none`).
- `effective` and `sources` never name an effort level.
- So taking an effort level away needs no restart for any level that was recorded. Still not visible: the level a
  model request carries.

## 9. "Turn on" of a server that cannot start; `mcp_reconnect` on a server whose program is gone

Question: the answer (success or error, which words) and the status after it.

Fixtures: `r7-mcp-cannot-start-1`, `r7-mcp-cannot-start-2`. A THROW-AWAY HOME (a switch is saved: section 1). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r7-mcp-cannot-start-1.json`. `ping-flaky` exits at once with code 1 while the file `switch.on` is missing; `no-such-command` names a program that does not exist. Each step is followed by a note (starts so far, which files exist, the saved `disabledMcpServers`) and `mcp_status`.

`mcp_toggle` on while the program exits at once, then `mcp_status` (first recording):

```json
{"type":"control_request","request_id":"host-4","request":{"subtype":"mcp_toggle","serverName":"ping-flaky","enabled":true}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-4","error":"Connection closed"}}
{"type":"control_request","request_id":"host-5","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-5","response":{"mcpServers":[{"name":"ping-flaky","status":"failed","error":"Connection closed","config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt","need=switch.on"]},"transport":"stdio","scope":"dynamic","source":"dynamic"},{"name":"no-such-command","status":"failed","error":"Executable not found in $PATH: \"stdio\"","config":{"type":"stdio","command":"codly-no-such-program-7","args":[]},"transport":"stdio","scope":"dynamic","source":"dynamic"}]}}}
```

`mcp_reconnect` after the program file was deleted, and the answers for the program that does not exist:

```json
{"type":"control_request","request_id":"host-8","request":{"subtype":"mcp_reconnect","serverName":"ping-flaky"}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-8","error":"Connection closed"}}
{"type":"control_request","request_id":"host-14","request":{"subtype":"mcp_toggle","serverName":"no-such-command","enabled":false}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-14"}}
{"type":"control_request","request_id":"host-18","request":{"subtype":"mcp_reconnect","serverName":"no-such-command"}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-18","error":"Executable not found in $PATH: \"stdio\""}}
```

| step | answer | `ping-flaky` after it | `no-such-command` after it | saved `disabledMcpServers` | `ping-flaky` starts so far |
|---|---|---|---|---|---|
| start + 3 s |  | `connected` | `failed` (`Executable not found in $PATH: "stdio"`) | `null` | 1 |
| mcp_toggle off | `success` | `disabled` | `failed` (`Executable not found in $PATH: "stdio"`) | `["ping-flaky"]` | 1 |
| mcp_toggle on while the program exits at once | `error: Connection closed` | `failed` (`Connection closed`) | `failed` (`Executable not found in $PATH: "stdio"`) | `[]` | 3 |
| mcp_reconnect after switch.on was made again | `success` | `connected` | `failed` (`Executable not found in $PATH: "stdio"`) | `[]` | 4 |
| mcp_reconnect after the program file was deleted | `error: Connection closed` | `failed` (`Connection closed`) | `failed` (`Executable not found in $PATH: "stdio"`) | `[]` | 4 |
| mcp_toggle off | `success` | `disabled` | `failed` (`Executable not found in $PATH: "stdio"`) | `["ping-flaky"]` | 4 |
| mcp_toggle on while the program file is gone | `error: Connection closed` | `failed` (`Connection closed`) | `failed` (`Executable not found in $PATH: "stdio"`) | `[]` | 4 |
| mcp_toggle off (no-such-command) | `success` | `failed` (`Connection closed`) | `disabled` | `["no-such-command"]` | 4 |
| mcp_toggle on (no-such-command) | `error: Executable not found in $PATH: "stdio"` | `failed` (`Connection closed`) | `failed` (`Executable not found in $PATH: "stdio"`) | `[]` | 4 |
| mcp_reconnect (no-such-command) | `error: Executable not found in $PATH: "stdio"` | `failed` (`Connection closed`) | `failed` (`Executable not found in $PATH: "stdio"`) | `[]` | 4 |

(`null`: the throw-away HOME had no entry for the folder yet. A start is a line the program wrote: a program file that
is gone or does not exist writes none.)


What the adapter can rely on (n=2, the two recordings agree step for step):
- A "Turn on" whose program cannot start is a control ERROR: `{"subtype":"error","error":"Connection closed"}` for a
  program that starts and exits at once, and for a program file that is gone (`python3` cannot open it). `mcp_status`
  then says `failed` with the same words in `error`. For the program that exits at once the CLI started it TWICE
  before it answered (starts 1 -> 3).
- `mcp_reconnect` on a server whose program is gone: the same error and the same status.
- A program that does not exist: `Executable not found in $PATH: "stdio"` in the answer and in `mcp_status`. The words
  name the transport type (`stdio`), not the program (part 1, `broken-mcp`, has the same quirk in another form). Do
  not show these words as the name of what is missing.
- A failed "Turn on" still counts as ON for the saved switch: `disabledMcpServers` is `[]` right after it. The next
  process in the folder will try to start the server again (and fail the same way).
- `mcp_reconnect` once the cause is gone (the file `switch.on` back) answers success and the server is `connected`.

## 10. The spending limit reached while a long tool call runs, and while a helper works

Question: is the call cut off, or does it run to its end? What happens to the helper? What does the stop say?

Fixtures: `r7-budget-during-call-1-calib`, `r7-budget-during-call-1-t1`, `r7-budget-during-call-2-calib`, `r7-budget-during-call-2-t1`. Flags of a try: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent --max-budget-usd 0.000563`. The cost grows only when a model request ends, and a running Bash call makes none, so the limit can only be passed DURING the call by a helper's request. One turn: the model starts a helper in the background that writes a story of about 600 words with no tool (one long request), then runs `echo go; sleep 61.5` in the foreground, then answers. Run `calib` has no budget; each try gets `--max-budget-usd` = the cost when the Bash call began + half the helper's cost, both from the run before. The script stops at the first try whose budget lay between those two costs, whose story came while the call ran, and whose turn ended with `error_max_budget_usd`. Each label needed one try.

From the Bash call's `tool_use` line to the end (`r7-budget-during-call-1-t1`):

```json
# t=3.32
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-2","name":"Bash","input":{"command":"echo go; sleep 61.5","description":"Print go then wait 61.5 seconds","timeout":120000}}],"…":"…"},"…":"…"}
# t=7.17
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"Print go then wait 61.5 seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=12.37
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"**The Keeper of Hollow Point**\n\nFor thirty-one years, Elin Marr climbed the one hundred an…"}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=12.37
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795472033962}}
# t=12.37
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r7-budget-during-call-1/sess-1/tasks/tid-1.output","summary":"**The Keeper of Hollow Point**\n\nFor thirty-one years, Elin Marr climbed the one hundred and twelve iron steps of Hollow Point every evening at dusk, and for thirty-one years the light had never once gone dark on her watch.\n\nShe was not sentimental about it. The lamp was a machine, she told the visiting inspectors, and …","usage":"…"}
# t=12.37
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=13.18
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":2,"stop_reason":"tool_use","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000563)"],"queued_turn_count":0,"result_index":0,"…":"…"}
# t=13.18
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=13.18
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":null,"terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000563)"],"origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=16.31
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: alive"},"parent_tool_use_id":null},"_t":16.313}
# t=16.31
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=16.31
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":null,"terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000563)"],"queued_turn_count":0,"result_index":2,"…":"…"}
# t=16.56
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":16.562}
# t=16.56
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795472038092}}
# t=16.56
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-budget-during-call-1/sess-1/tasks/tid-2.output","summary":"Print go then wait 61.5 seconds"}
# t=17.31
{"_meta":{"exit_code":1},"_t":17.309}
# t=17.35
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":["system/task_updated \"killed\"","system/task_notification \"stopped\""],"sleep_61_5_alive":false}},"_t":17.353}
```

| try | budget (USD) | cost when the call began | cost once the story was written | story line (s) | `result` (s) | call's result | `sleep` alive at the result / 3 s later / after one more message | after stdin close |
|---|---|---|---|---|---|---|---|---|
| `r7-budget-during-call-1-t1` | 0.000563 | 0.000504 | 0.001302 | 12.37 | 13.18 | none | yes / yes / yes | `system/task_updated "killed"`, `system/task_notification "stopped"` |
| `r7-budget-during-call-2-t1` | 0.001674 | 0.001349 | 0.002057 | 10.14 | 10.87 | none | yes / yes / yes | `system/task_updated "killed"`, `system/task_notification "stopped"` |

What the adapter can rely on (n=2):
- The turn is cut off, NOT the call. 0.81, 0.72 s after the helper's request that passed the limit ended (its story line), the
  turn's `result` came: `error_max_budget_usd`, `terminal_reason:"budget_exhausted"`, `stop_reason:"tool_use"`,
  `errors:["Reached maximum budget ($<budget>)"]`. The Bash call had no tool result.
- The CALL RUNS ON, unseen: its `sleep 61.5` was alive at the `result`, 3 s later and after one more message; it is
  not in `background_tasks_changed` (the list was `[]`). It ended only when stdin was closed: `task_updated` `killed`
  and `task_notification` `stopped` for its task (a foreground Bash call that had become a task: section 12), then the
  exit. (In the runs with no budget the same call ran its 61.5 s and returned.)
- The helper's request that passed the limit was its last: the helper ended `completed` (`task_updated`,
  `task_notification`) before the turn's `result`. So "reached while a helper works" ends no helper here; a helper
  that still had requests to make is not recorded.
- Right after the stop the CLI starts a turn by itself for the helper's notification, and that turn fails at once
  with the same `result` (`num_turns` 1, `origin.kind:"task-notification"`, no model request). One more message fails
  the same way (part 4, section 10: the process does no more work).
- For the stop text: the work of the turn is cut off, but a command it started may still run; after a budget stop the
  host should close the process (stdin) so that such a command ends.

## 11. `stop_task` on a background helper WHILE a turn of the agent runs

Question: does a turn start by itself after the running one?

Fixtures: `r7-stop-helper-mid-turn-1`, `r7-stop-helper-mid-turn-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. Turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 173; echo end`) and ends. Turn 2 runs `echo go; sleep 21.5`; 2 s into that call the host sends `stop_task` with the helper's id, reads until turn 2's result and 20 s more.

Every line from the driver's note before `stop_task` to the end (first recording):

```json
# t=6.80
{"_meta":{"before_stop_task":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run begin/sleep/end echo"}],"a_turn_runs":true,"sleep_173_alive":true,"sleep_21_5_alive":true}},"_t":6.805}
# t=6.80
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":6.805}
# t=6.80
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795470567341}}
# t=6.80
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r7-stop-helper-mid-turn-1/sess-1/tasks/tid-1.output","summary":"Run begin/sleep/end echo"}
# t=6.80
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=6.80
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=6.80
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Print begin, sleep 173 seconds, print end"}
# t=6.80
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=6.80
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=26.22
{"type":"system","subtype":"task_notification","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-3","status":"completed","output_file":"","summary":"Echo go then sleep 21.5 seconds"}
# t=26.22
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-3","type":"tool_result","content":"go","is_error":false}]},"…":"…"}
# t=27.99
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"thinking","…":"…"}],"…":"…"},"…":"…"}
# t=27.99
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"done"}],"…":"…"},"…":"…"}
# t=27.99
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"done","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
# t=48.24
{"_meta":{"after_stop_task":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(10 more)"],"results_since_the_request":1,"result_origins":[null],"list":[],"sleep_173_alive":false,"sleep_21_5_alive":false},"seconds_from_stop_task":{"answer":0.007,"task_notification":0.007,"result":21.221,"init_after_it":null}},"_t":48.24}
# t=48.24
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":48.24}
# t=48.88
{"_meta":{"exit_code":0},"_t":48.884}
# t=48.89
{"_meta":{"transcript":{"transcript_found":true,"enqueued":1,"attached_to_a_running_turn":1,"removed":1,"dequeued":0}},"_t":48.887}
```

What the adapter can rely on (n=2):
- The stop itself is as with no turn running: `task_updated` `killed`, `task_notification` `stopped`, the list `[]`,
  the answer `{}`, then the helper's two `user` lines.
- The running turn goes on and ends normally (`success`). NO turn starts by itself after it: no `system/init` in the
  20 s after that `result`. (With no turn running the CLI starts one at once: part 3, section 1; part 4, section 2.)
- Where the helper's notification went: the session's transcript shows ONE task notification put in the CLI's queue
  and handed to a model request of the running turn (`queued_command`), then taken out of the queue, in each run. So
  the agent is told about the stop inside the turn that runs; the stream itself shows no line for that.
- So the hint "goes on with a turn of its own" is right only when no turn runs. When a turn runs, the agent is told
  within that turn.

## 12. A foreground Bash call that becomes a task and ends by itself

Question: its `task_notification` (status word, `output_file`); how long a call must run before it becomes a task.

Fixtures: `r7-bash-becomes-task-1`, `r7-bash-becomes-task-2`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`. One process, one turn per call: `echo q12; sleep D` for D = 1, 2, 3, 4, 6, 9, 14 s, in the foreground. The commands end by themselves.

The call of 4 s, from its `tool_use` line to its tool result (first recording):

```json
# t=12.75
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-4","name":"Bash","input":{"command":"echo q12; sleep 4","description":"Print marker and wait four seconds"}}],"…":"…"},"…":"…"}
# t=15.94
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-4","description":"Print marker and wait four seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=16.95
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-4","status":"completed","output_file":"","summary":"Print marker and wait four seconds"}
# t=16.95
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-4","type":"tool_result","content":"q12","is_error":false}]},"…":"…"}
```

| D (s) | a task? run 1 / run 2 | `tool_use` -> `task_started` (s) | `tool_use` -> tool result (s) | `task_notification` |
|---|---|---|---|---|
| 1 | no / no | - / - | 2.048 / 2.625 | none |
| 2 | no / no | - / - | 2.031 / 2.184 | none |
| 3 | yes / yes | 3.008 / 3.015 | 3.030 / 3.036 | `completed`, `"output_file":""` |
| 4 | yes / yes | 3.006 / 3.012 | 4.029 / 4.043 | `completed`, `"output_file":""` |
| 6 | yes / yes | 3.007 / 3.024 | 6.032 / 6.065 | `completed`, `"output_file":""` |
| 9 | yes / yes | 3.011 / 3.036 | 9.642 / 9.135 | `completed`, `"output_file":""` |
| 14 | yes / yes | 3.021 / 3.023 | 14.079 / 14.155 | `completed`, `"output_file":""` |

What the adapter can rely on (n=2 per duration; 10 calls became tasks, 4 did not):
- A foreground Bash call becomes a task about 3 s after its `tool_use` line: `task_started` came 3.006 to 3.036 s after it,
  with `"task_type":"local_bash"`, `"is_backgrounded":false` and the call's `tool_use_id`. The calls of D = 1 and 2
  (tool results 2.031 to 2.625 s after the line) got no task line. The 3 s calls became tasks about 0.02 s before they
  returned.
- When the call ends by itself: ONE `task_notification` with `"status":"completed"`, `"output_file":""` and the call's
  description in `summary`, written just BEFORE the call's tool result. No `task_updated`, no list line (it was never
  in the background list).
- The same inside a helper (section 3): the helper's Bash call has `"owned_by_subagent":true`, and its end is one
  `task_notification` `completed` with an empty `output_file`.
- So such a line belongs to a Bash call (its `tool_use_id` is the call's), not to a helper or a background command; it
  must end neither. Status words of these lines seen so far: `completed` (here), `stopped` (an interrupt or a stop:
  part 4, sections 1 and 2; the budget stop's close: section 10).

## Notes of part 4 written late

The script `r6_a2_shutdown_helper.py` listed the lines, then looked for two processes, then wrote its note `before_stdin_is_closed`; a line that came in between is in the file before the note but not in its list. The other part 4 notes that list lines (`r6_a1`: `after_the_interrupt`; `r6_a2`: `after_stop_task`, `after_the_exit`) were made in the same order. This build compares each such list with the lines in the fixture between the note's starting point (the first host request; for `after_the_exit` the close of stdin) and the note:

| fixture | note | lines in its list | lines in the file before it |
|---|---|---|---|
| `r6-shutdown-helper-idle-close-now-1` | `before_stdin_is_closed` | 4 | 7 |
| `r6-shutdown-helper-idle-close-now-2` | `before_stdin_is_closed` | 4 | 7 |

Every other such note of part 4 lists exactly the lines in its window. The two late notes are the ones part 4 quotes in
section 2 (`idle-close-now`): there the `user`, `user` and `system/init` lines of the self-started turn came between
the list and the note, so the note's `init_of_a_turn` is null although that `init` came before stdin was closed. What
part 4 says about that variant (the turn had begun when stdin was closed, and ran to its end after it) follows the
lines of the file, not the note, and stands. Part 4 now says this under its quote. `r6_a2` is fixed (process checks
first, then the list and the note in one step under the driver's lock); its fixtures were not recorded again.

## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | The hint and the record of "Turn off" (`live_session.rs` "for <agent>'s run"; `session.ts` `MCP_HINT`) | yes: the switch is saved in `~/.claude.json` under the folder's path (`disabledMcpServers`) and holds for every later process in that folder, new or resumed, until "Turn on". The words "for <agent>'s run" are wrong: it is for this folder. "Turn on starts it again" is right |
| 2 | Dropping `--strict-mcp-config` so that your own servers work in a folder you have not trusted (`trust.rs`, `claude.rs` `use_user_setup`) | yes, for the repo's `.mcp.json`: not loaded in 16 of 16 recordings of parts 4 and 5 under `user` or `user,local` (4 logged in), also with an approval kept in `~/.claude.json`. To know: with `user,local` a local-scope server of the folder (the user's own) loads too. Not recorded: plugins' servers |
| 3 | `on_result` and the store's rule for a FAILED turn (helpers) | yes: a turn that fails by a limit ends no background helper; the helper ends at its own `task_notification`, and a turn by itself follows. Keep helpers past a failed turn, as past a done one. Not recorded: a failure by an API error |
| 4 | The same rule for an interrupt with several helpers, or a helper that waits for the model | yes: each helper gets its own `killed` / `stopped` pair before the answer (2 helpers, n=2; a helper waiting for its answer, n=2) |
| 5 | `begin_shutdown` / `wind_down` | yes: `interrupt` first, also with no turn running (it ends the helpers and leaves the commands listed), then `stop_task` for each listed command, then close: exit in 0.70 s, nothing left (n=2). Today `wind_down` with no turn running skips the interrupt and leaves a helper to the later steps |
| 6 | The watchdog's short limit for `mcp__` tools | yes: an HTTP server's tool gets the same 30 s heartbeat. New: such a call ends with `The operation timed out.` at 60 s (n=2, a server that answers with a JSON body): a limit above 60 s for it is never reached |
| 7 | One answer for "approve and accept edits" (`grantable` never sends `setMode`) | yes: the request's own `setMode` suggestion in the allow answer switches the session to `acceptEdits` (status line at once, next `init`, the next Edit does not ask). Read the mode back from those lines: `get_settings` has none |
| 8 | Taking an effort away with no restart (`runs.rs` `live_model_switch`) | yes: `effortLevel:null` gives `medium` for `low`, `high` and `max` (n=2 each) |
| 9 | The toast of a failed "Turn on" / "Restart" (`AgentMcp.svelte`) | yes: both are control errors with the CLI's words (`Connection closed`; `Executable not found in $PATH: "stdio"`, which names the transport, not the program) and `mcp_status` says `failed` with the same words. After a failed "Turn on" the saved switch is on |
| 10 | The text of the budget stop (`failure_reason`) and package `answers-and-limits` item 3 | partly: the text can say that the work stopped (`budget_exhausted`, `Reached maximum budget ($…)`); it must not say that everything stopped: a Bash call that runs at that moment runs on, unlisted, until the process ends. The host should end the process after such a stop. A helper with requests left at the stop is not recorded |
| 11 | The Stop hint of a helper row (`session.ts` `stopHint`) and the demo | yes: the agent "goes on with a turn of its own" only when no turn runs; during a turn it is told within that turn and no turn follows |
| 12 | `on_task` (a line that must end no helper and no background row) | yes: a foreground Bash call becomes a task about 3 s after its `tool_use`; its end by itself is one `task_notification` `completed` with `"output_file":""` and the call's `tool_use_id`, no `task_updated`, no list line |

## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | `mcp_toggle` off, exit, a new process in the same folder | Saved in `~/.claude.json` (`projects[<folder>].disabledMcpServers`); new and resumed processes say `disabled`, do not start the server; only `mcp_toggle` on undoes it | 2 | `r7-mcp-toggle-saved-1-a` … `-2-c` |
| 2 | The repo's `.mcp.json` without `--strict-mcp-config` under `user` / `user,local` | Never loaded (10 recordings here, 6 in part 4), also with an approval in `~/.claude.json`; a local-scope server loads with `user,local` only | 1 to 2 per form | `r7-untrusted-mcp-*` |
| 3 | A turn that fails by itself while a helper works | The helper goes on (still listed), ends `completed` later, a turn by itself follows | 2 | `r7-failed-turn-helper-1`, `-2` |
| 4 | An interrupt with two helpers; with a helper that waits for the model | All ended: `killed` / `stopped` for each before the answer; the waiting helper's line is `[Request interrupted by user]` | 2 + 2 | `r7-interrupt-two-helpers-1`, `-2`, `r7-interrupt-helper-model-1`, `-2` |
| 5 | Shutdown with a helper and a command | interrupt ends the helper (command still listed), `stop_task` ends the command, exit 0.70 s after close, nothing left | 2 | `r7-shutdown-helper-and-command-1`, `-2` |
| 6 | `tool_progress` of an HTTP MCP tool | Heartbeat at 30 and 60 s, as for stdio; the call ends at 60 s with `The operation timed out.` | 2 (+1 control) | `r7-tool-progress-http-70-1`, `-2`, `-45-1` |
| 7 | Allow with the request's `setMode` suggestion | Session in `acceptEdits`: status line, next `init`, next Edit not asked; `get_settings` has no mode | 2 | `r7-allow-set-mode-1`, `-2` |
| 8 | `effortLevel:null` on `--effort low` / `max` | `medium` at once and after a turn | 2 per level | `r7-effort-null-low-1` … `-max-2` |
| 9 | "Turn on" of a server that cannot start; reconnect of a gone program | control error `Connection closed` (or `Executable not found in $PATH: "stdio"`), status `failed` with the same words; the saved switch is on | 2 | `r7-mcp-cannot-start-1`, `-2` |
| 10 | Budget reached during a long call and while a helper works | The turn ends (`budget_exhausted`), the call runs on unlisted until stdin closes; the helper had completed; a turn by itself fails at once | 2 | `r7-budget-during-call-1-calib`, `-1-t1`, `-2-calib`, `-2-t1` |
| 11 | `stop_task` on a helper while a turn runs | The turn goes on; no turn by itself after it; the notification goes into the running turn | 2 | `r7-stop-helper-mid-turn-1`, `-2` |
| 12 | A Bash call that becomes a task and ends by itself | Task about 3 s after `tool_use`; `task_notification` `completed`, `"output_file":""`, before the tool result, no `task_updated` | 2 per duration | `r7-bash-becomes-task-1`, `-2` |

## Not verified

Not in this part (still open from the earlier lists):
- The text printed when `failIfUnavailable` stops a start (needs a machine where the sandbox cannot start).
- `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url` (a real sign-in; may write to the login keychain).
- `claude --resume` typed into the app's terminal on a session Codly started headless.
- Linux behaviour of process groups.
- A `tool_progress` line for WebFetch (needs a slow https page with a trusted certificate).
- A network allow that lasts hours (part 3 recorded 304 s).
- Whether a model request carries the effort level that `get_settings` shows.
- The `can_use_tool` request of a user-scope server (needs the user's real setup).

Open after these recordings:
- 1: whether a REAL HOME behaves the same (recorded in a throw-away HOME: the same file layout is assumed); a server of
  the same name in another folder (expected: not touched, its own entry); a switch made while a turn runs.
- 2: a server from a plugin; the logged-in forms a third time; `user,local` with a local file that has only one of the
  two keys.
- 3: a turn that fails by an API error, or by the budget while a helper still has requests to make.
- 4: a nested helper; a helper with a background command of its own; `interrupt` with `cancel_queued` or `send_now`
  while helpers work.
- 5: the same with a turn running; a helper that ignores the stop (none did).
- 6: an HTTP server that streams its answer (event stream) or sends progress notifications: does the 60 s limit move?
  Whether a setting changes that limit.
- 7: the same answer on a Bash request (part 1 had it among several suggestions); a `setMode` to another mode.
- 8: `null` while a turn runs.
- 9: a server that cannot start at the process start AND is then turned off and on in a NEW process (the saved switch).
- 10: a helper that still had requests to make when the limit was passed; whether closing stdin is the only end of the
  call that runs on (an `interrupt` after the stop was not tried).
- 11: `stop_task` on a COMMAND while a turn runs.
- 12: the line of a Bash call that becomes a task and FAILS by itself (exit code not 0).
