# Claude Code CLI stream-json: recorded protocol shapes, part 4

- CLI: `claude` 2.1.311 (the same version as parts 1 to 3; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 and 3. Fixtures of this part are `fixtures/claude-2.1.311/r6-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r6_*.py` (README there says how to record again).
- List price of the 63 recordings (sum of `total_cost_usd` of their `result` lines; a session that several recordings
  resume is counted once): 0.13 USD. 0.05 USD of it is section 5, the one scenario that leaves haiku on purpose (one
  tiny turn on sonnet in each of its two recordings).
- Everything below was observed in these recordings. Each answer says how many runs it rests on (n) and says no more
  than the recordings show.
- The sections are numbered as the 11 questions this part was recorded for; the last table has them in the same order.

Differences from the part 3 method:
- TIMES. Every time in this part is a time stamp of the recording, not a time the driver measured around a wait. The
  driver's reader thread stamps each stdout line when it reads it and each request when it writes it; the notes named
  `seconds_from_…` are differences of those stamps. (The driver's waits look for a line every 0.2 s. Part 3, section 4,
  gave such a wait as a latency: that text is corrected, see "Corrections to part 3" below.) The moment the process is
  gone is a note of its own (`process_exited`, written by a thread that waits for the process).
- A scenario that stops a running command waits until it SEES the command's process (`sleep N`), then the stated
  seconds. The `tool_use` line of a call can come seconds before its process exists.
- Five recordings of section 8 run with the REAL login AND the user's own settings (`--setting-sources user…`, no
  `--strict-mcp-config`), because that is the question. For these the run adds `"disableAllHooks":true` to
  `--settings` (so the user's hooks do not run; checked first with a throw-away HOME, `r6-project-mcp-hooks-on` and
  `-hooks-off`), turns the claude.ai connectors off, allows only the scenario's one tool and denies every other
  request, and sends no `mcp_status`. Servers of the user's own setup are removed from the fixtures (a note says
  whether there were any), and the name of the user's output style is a placeholder.
- A THROW-AWAY HOME (as in part 3) has no login: a turn ends at once with `Not logged in · Please run /login` and
  costs nothing. Such runs show what is LOADED, not a tool call.
- New sanitizing: the MCP server `ping-live` is kept in `init` and `mcp_status`.
- In quoted lines `"…":"…"` stands for keys left out; `# t=` lines are seconds since the process started. A quoted
  line of a subagent keeps `parent_tool_use_id` and `agent_id`.

## 1. An `interrupt` while a helper (subagent) works

Question: (a) a FOREGROUND helper (its `Agent` call still waits): which lines end it, with which status word? (b) a BACKGROUND helper: does it go on, is it still in `background_tasks_changed`, what ends it later?

Fixtures: `r6-interrupt-helper-foreground-1`, `r6-interrupt-helper-foreground-2` (a), `r6-interrupt-helper-background-1`, `r6-interrupt-helper-background-2` (b, while another turn runs), `r6-interrupt-helper-idle-1`, `r6-interrupt-helper-idle-2` (b, while no turn runs); each scenario twice. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. The recordings of section 2 start the same way and are counted where they show the same thing.

### (a) The helper's `Agent` call still waits

One turn: the model starts a subagent with `run_in_background:false`; the subagent's Bash call runs `echo begin; sleep 127; echo end`. 3 s into that call the host sends `interrupt`, reads for 10 s, sends `stop_task` with the helper's task id, and one more message.

```json
{"type":"user","session_id":"","message":{"role":"user","content":"Use the Agent tool with run_in_background set to false to have a subagent run the Bash command `echo begin; sleep 127; echo end` and report its output. Then tell me what it reported."},"parent_tool_use_id":null}
```

The task lines before the interrupt, then every line from the driver's note before the interrupt to the end (first recording):

```json
# t=3.36
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run timed echo/sleep command","subagent_type":"general-purpose","is_backgrounded":false,"spawn_depth":1,"task_type":"local_agent","prompt":"Run this exact Bash command and report its full output verbatim, nothing else:\n\necho begin; sleep 127; echo end\n\nUse the Bash tool. If it fails or times out, report the exact error."}
# t=11.91
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","owned_by_subagent":true,"tool_use_id":"toolu-2","description":"Print begin, sleep 127 seconds, print end","is_backgrounded":false,"parent_task_id":"tid-1","task_type":"local_bash"}
# t=14.40
{"_meta":{"before_the_interrupt":{"list":[],"helper_is_backgrounded":false,"sleep_127_alive":true}},"_t":14.4}
# t=14.40
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":14.4}
# t=14.40
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=14.40
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Print begin, sleep 127 seconds, print end"}
# t=14.40
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-1"}]},"…":"…"}
# t=14.40
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795460706585}}
# t=14.40
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-interrupt-helper-foreground-1/sess-1/tasks/tid-1.output","summary":"Run timed echo/sleep command"}
# t=14.40
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=14.40
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":0,"…":"…"}
# t=24.72
{"_meta":{"after_the_interrupt":{"result_came":true,"lines":["control_response success","system/task_notification \"stopped\"","user","system/task_updated \"killed\"","system/task_notification \"stopped\"","user","result/error_during_execution"],"results_in_the_10s_after":1,"sleep_127_alive":false,"list":[]},"seconds_from_the_interrupt_request":{"answer":0.01,"task_updated":0.032,"task_notification":0.016,"result":0.044,"next_init":null}},"_t":24.723}
# t=24.72
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":24.723}
# t=24.72
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{}}}
# t=24.73
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: alive"},"parent_tool_use_id":null},"_t":24.733}
# t=24.73
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=25.20
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"alive"}],"…":"…"},"…":"…"}
# t=25.20
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"alive","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
# t=27.64
{"_meta":{"at_the_end":{"sleep_127_alive":false,"list":[],"results":2}},"_t":27.645}
# t=27.64
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":27.645}
# t=28.57
{"_meta":{"exit_code":0},"_t":28.567}
```

The line that rejects the `Agent` call, with its `tool_result_meta`:

```json
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-1"}]},"tool_use_result":"User rejected tool use","tool_result_meta":[{"id":"toolu-1","non_execution_kind":"user-rejected"}],"…":"…"}
```

What the adapter can rely on (n=2, the two recordings have the same lines in the same order):
- The interrupt ends the foreground helper together with the turn. In order: the answer to `interrupt`; a
  `task_notification` for the helper's Bash call (see the next point); the `user` line that rejects the `Agent` call
  (the usual "doesn't want to proceed" tool result, `tool_result_meta` with `"non_execution_kind":"user-rejected"`);
  `task_updated` with `patch.status:"killed"` and `task_notification` with `"status":"stopped"` for the helper; the
  `user` line `[Request interrupted by user for tool use]`; the turn's `result` (`error_during_execution`,
  `terminal_reason:"aborted_tools"`).
- A Bash call that has run for a while is a TASK OF ITS OWN: `task_started` with `"task_type":"local_bash"`,
  `"is_backgrounded":false`, and inside a helper `"owned_by_subagent":true` and `"parent_task_id":<the helper's task
  id>` (part 3, section 2). When the interrupt comes it gets a `task_notification` of its own: `"status":"stopped"`,
  `"output_file":""`, its own `task_id` and `tool_use_id`, and NO `task_updated`. A host that takes every
  `task_notification` for the end of a helper or of a background command is wrong: look the `task_id` up.
- So the status words of a helper that an interrupt ends are the same as after `stop_task`: `killed` in
  `task_updated`, `stopped` in `task_notification`. The task lines come BEFORE the turn's `result`.
- No `background_tasks_changed` line: a foreground helper was never in the list (part 3, section 2).
- No line of the helper itself follows (no `user` line with `parent_tool_use_id`): unlike a background helper (below),
  the rejection of its running Bash call is not written.
- The helper's `sleep 127` was gone when the host looked 10 s later, and no turn started by itself in those 10 s.
- From the request to the `result`: 0.044, 0.040 s (time stamps of the lines).
- `stop_task` with the helper's task id afterwards answers `{}` and writes nothing. The next turn works.

### (b) A background helper, while another turn runs

Turn 1 starts a subagent with `run_in_background:true` (its Bash call: `echo begin; sleep 43; echo end`) and ends. Turn 2 runs `echo go; sleep 59` in the foreground. 3 s into that call the host sends `interrupt` and reads for 10 s; then one more message. The task lines before the interrupt, then every line from the driver's note before the interrupt to the note after it (first recording):

```json
# t=2.24
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run delayed echo command","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent","prompt":"Run the Bash command `echo begin; sleep 43; echo end` and report its output."}
# t=13.87
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","owned_by_subagent":true,"tool_use_id":"toolu-2","description":"Print begin, sleep 43 seconds, print end","is_backgrounded":false,"parent_task_id":"tid-1","task_type":"local_bash"}
# t=14.90
{"type":"system","subtype":"task_started","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-3","description":"Echo go then sleep 59 seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=15.81
{"_meta":{"before_the_interrupt":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run delayed echo command"}],"a_turn_runs":true,"sleep_43_alive":true,"sleep_59_alive":true}},"_t":15.806}
# t=15.81
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":15.807}
# t=15.81
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795460775885}}
# t=15.81
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-interrupt-helper-background-1/sess-1/tasks/tid-1.output","summary":"Run delayed echo command"}
# t=15.81
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=15.81
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=15.81
{"type":"system","subtype":"task_notification","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-3","status":"stopped","output_file":"","summary":"Echo go then sleep 59 seconds"}
# t=15.81
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-3"}]},"…":"…"}
# t=15.81
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=15.81
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Print begin, sleep 43 seconds, print end"}
# t=15.81
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=15.81
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=15.81
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=26.62
{"_meta":{"after_the_interrupt":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(9 more)"],"results_in_the_10s_after":1,"sleep_43_alive":false,"sleep_59_alive":false,"list":[]},"seconds_from_the_interrupt_request":{"answer":0.022,"task_updated":0.022,"task_notification":0.022,"result":0.051,"next_init":null}},"_t":26.62}
```

### (b) A background helper, while NO turn runs

The same turn 1. No second turn: 3 s into the helper's Bash call the host sends `interrupt` and reads for 10 s; then one more message. The task lines before the interrupt, then every line from the driver's note before the interrupt to the note after it (first recording):

```json
# t=4.96
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run sleep echo test","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent","prompt":"Run the Bash command `echo begin; sleep 43; echo end` and report its output."}
# t=15.86
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","owned_by_subagent":true,"tool_use_id":"toolu-2","description":"Print begin, sleep 43 seconds, print end","is_backgrounded":false,"parent_task_id":"tid-1","task_type":"local_bash"}
# t=16.61
{"_meta":{"before_the_interrupt":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run sleep echo test"}],"a_turn_runs":false,"sleep_43_alive":true,"sleep_59_alive":false}},"_t":16.612}
# t=16.61
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":16.612}
# t=16.61
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795460861302}}
# t=16.61
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-interrupt-helper-idle-1/sess-1/tasks/tid-1.output","summary":"Run sleep echo test"}
# t=16.61
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=16.61
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=16.61
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Print begin, sleep 43 seconds, print end"}
# t=16.61
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=16.61
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=27.67
{"_meta":{"after_the_interrupt":{"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","system/background_tasks_changed 0 listed","control_response success","system/task_notification \"stopped\"","user (helper)","user (helper)"],"results_in_the_10s_after":0,"sleep_43_alive":false,"sleep_59_alive":false,"list":[]},"seconds_from_the_interrupt_request":{"answer":0.039,"task_updated":0.038,"task_notification":0.038,"result":null,"next_init":null}},"_t":27.666}
```

What the adapter can rely on (with a turn: n=4, the two recordings here and the two `sequence` recordings of section 2;
without a turn: n=4, the two here and the two `idle-interrupt` recordings of section 2; the eight agree):
- A background helper does NOT go on after an `interrupt`. The interrupt ends it, whether a turn runs or not:
  `task_updated` (`patch.status:"killed"`), `task_notification` (`"status":"stopped"`), `background_tasks_changed`
  with `[]`, and only THEN the answer to `interrupt`. From the request to the answer: 0.022, 0.039, 0.039, 0.089 s.
- So after an interrupt the helper is no longer in the list, and nothing "ends it later": it is over. Its running Bash
  call is rejected (two `user` lines with `parent_tool_use_id` and `agent_id`: the "doesn't want to proceed" tool
  result and `[Request interrupted by user for tool use]`), and its `sleep 43` was gone when the host looked.
- With a turn running, the turn's own call is rejected too and the turn's `result` is the usual
  `error_during_execution`. The task lines and the answer came first and the `result` last in all four recordings.
  Between them the turn's two `user` lines came before the helper's two in all four; four recordings are not enough
  to rely on that order.
- With no turn running there is no `result` line at all: the answer to `interrupt` is the last line the host can
  wait for (the helper's two `user` lines follow it).
- Each Bash call that had become a task of its own gets its `task_notification` (`stopped`, `"output_file":""`, no
  `task_updated`) after the answer: the helper's call, and the turn's own call when one ran. Whether a call is a
  task yet depends on how long it has run: in 11 of the 18 recordings of sections 1 and 2 at least one call was, in
  the others the stop came first and no such line was written. The builder of this document checks for all 18 that
  exactly the calls with a `task_started` before the stop get this line.
- Different from `stop_task` on the same helper (part 3, section 1): after the interrupt NO turn starts by itself (none
  in the 10 s the host read, in the four recordings of this section).
- Different from a background COMMAND (`local_bash` with `is_backgrounded:true`): part 3, section 4, has one that an
  interrupt left running and listed (it needed `stop_task`). An interrupt ends helpers and foreground calls, not
  background commands.
- For the host: a turn that ends as interrupted takes EVERY helper with it, background ones too, and the stream says so
  itself: `task_notification` `stopped` for each helper, with the `tool_use_id` of its `Agent` call, and the list
  line. In the six recordings with a turn these lines come before the turn's `result`. So a host that ends a helper
  at the helper's own `task_notification` has no helper left open when the interrupted turn ends, and needs no rule
  of its own for it. (Only a host that did not follow that line and kept a background helper as "still at work"
  would be wrong.)
- What "EVERY helper" rests on: each recording of this part has ONE helper, always in the middle of a Bash call.
  Part 5, section 4, adds TWO background helpers at once (each ended with its own `task_updated` / `task_notification`
  pair, the list going from 2 to 1 to 0, all before the answer; n=2) and a helper that waited for its model answer,
  with no Bash call (ended the same way, its one `user` line is `[Request interrupted by user]`; n=2). A nested
  helper and a helper with a background command of its own are still not recorded.

## 2. The stop of a helper inside a shutdown; `stop_task` on a foreground helper

Question: as one run: `interrupt`, `stop_task` on the background helper, the turn Claude Code then starts by itself, stdin close. What must the host do so that the process exits, how long does it take, is anything left running? And `stop_task` on a helper whose `Agent` call still waits.

Fixtures: `r6-shutdown-helper-<variant>-1` and `-2` for the variants `sequence`, `idle-interrupt`, `idle-close-now`, `idle-interrupt-again`, `idle-outwait`, `foreground-stop` (each twice). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent`. The CLI is the leader of its own process group, as the app starts it.

All variants but the last: turn 1 starts a subagent in the background (its Bash call: `echo begin; sleep 139; echo end`) and ends. The host waits until it sees the `sleep 139` process, and 2 s more. Then:

### `sequence`: a turn runs

Every line from the driver's note before the interrupt (first recording):

```json
# t=10.03
{"_meta":{"before_the_stop":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run delayed echo command"}],"a_turn_runs":true,"sleep_139_alive":true,"sleep_67_alive":true}},"_t":10.032}
# t=10.03
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":10.032}
# t=10.03
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795460932181}}
# t=10.03
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-shutdown-helper-sequence-1/sess-1/tasks/tid-1.output","summary":"Run delayed echo command"}
# t=10.03
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=10.03
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=10.03
{"type":"system","subtype":"task_notification","task_id":"tid-3","run_id":"run-3","tool_use_id":"toolu-3","status":"stopped","output_file":"","summary":"Echo go and sleep 67 seconds"}
# t=10.03
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-3"}]},"…":"…"}
# t=10.03
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=10.03
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Print begin, sleep 139 seconds, print end"}
# t=10.03
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=10.03
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=10.03
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=10.43
{"_meta":{"after_the_interrupt":{"list":[],"sleep_139_alive":false,"sleep_67_alive":false}},"_t":10.432}
# t=10.43
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":10.432}
# t=10.43
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{}}}
# t=10.56
{"_meta":{"before_stdin_is_closed":{"lines_since_the_first_request":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(10 more)"],"list":[],"sleep_139_alive":false,"sleep_67_alive":false},"seconds_from_the_interrupt":{"answer":0.008,"task_notification":0.008,"result":0.032},"seconds_from_stop_task":{"answer":0.002,"task_notification":null,"init_of_a_turn":null,"result_of_a_turn":null}},"_t":10.563}
# t=10.56
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":10.564}
# t=11.30
{"_meta":{"process_exited":true},"_t":11.299}
# t=11.63
{"_meta":{"exit_code":1},"_t":11.632}
# t=12.72
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":[],"sleep_139_alive":false,"sleep_67_alive":false}},"_t":12.72}
```

### `idle-close-now`: `stop_task` with no turn running, stdin closed at once

Every line from the driver's note before `stop_task` (first recording):

```json
# t=9.91
{"_meta":{"before_the_stop":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"general-purpose","description":"Run sleep echo test in background"}],"a_turn_runs":false,"sleep_139_alive":true,"sleep_67_alive":false}},"_t":9.915}
# t=9.91
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":9.915}
# t=9.91
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795460999889}}
# t=9.91
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-shutdown-helper-idle-close-now-1/sess-1/tasks/tid-1.output","summary":"Run sleep echo test in background"}
# t=9.91
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=9.91
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=9.91
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=9.91
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=9.91
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=10.07
{"_meta":{"before_stdin_is_closed":{"lines_since_the_first_request":["system/task_updated \"killed\"","system/task_notification \"stopped\"","system/background_tasks_changed 0 listed","control_response success"],"list":[],"sleep_139_alive":true,"sleep_67_alive":false},"seconds_from_stop_task":{"answer":0.005,"task_notification":0.005,"init_of_a_turn":null,"result_of_a_turn":null}},"_t":10.075}
# t=10.08
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":10.076}
# t=10.92
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"The background subagent was stopped before it reported output, so I don't have the `begin`/`end` result. It was stopped …"}],"…":"…"},"…":"…"}
# t=10.92
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The background subagent was stopped before it reported output, so I don't have the `begin`/`end` result. It was stopped …","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=11.68
{"_meta":{"process_exited":true},"_t":11.68}
# t=12.03
{"_meta":{"exit_code":0},"_t":12.031}
# t=13.23
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":["assistant","assistant","result/success"],"sleep_139_alive":false,"sleep_67_alive":false}},"_t":13.229}
```

The note `before_stdin_is_closed` above is older than its time stamp: the driver listed the lines, then looked for two processes, then wrote the note, and the `user`, `user` and `system/init` lines came in between (they are in the file before the note; its `init_of_a_turn` is null and its list has 4 lines). Stdin was closed after that `init`. Part 5 ("Notes of part 4 written late") lists every note of part 4 this touches; the script is fixed.

### `idle-interrupt-again`: the self-started turn is interrupted

From `stop_task` on (first recording):

```json
# t=11.50
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":11.501}
# t=11.50
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795461033282}}
# t=11.50
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-shutdown-helper-idle-interrupt-again-1/sess-1/tasks/tid-1.output","summary":"Run sleep echo test in background"}
# t=11.50
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=11.50
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=11.50
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=11.50
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=11.50
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Task","Bash"],"mcp_servers":[],"…":"…"}
# t=11.59
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"interrupt"}},"_t":11.586}
# t=11.59
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"still_queued":[]}}}
# t=11.59
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"…":"…"}
# t=11.59
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":2,"stop_reason":null,"terminal_reason":"aborted_streaming","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"],"origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=11.91
{"_meta":{"before_stdin_is_closed":{"lines_since_the_first_request":["system/task_updated \"killed\"","system/task_notification \"stopped\"","…(8 more)"],"list":[],"sleep_139_alive":false,"sleep_67_alive":false},"seconds_from_the_interrupt":{"answer":0.007,"result":0.02},"seconds_from_stop_task":{"answer":0.009,"task_notification":0.009,"init_of_a_turn":0.085,"result_of_a_turn":0.105}},"_t":11.913}
# t=11.91
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":11.913}
# t=12.58
{"_meta":{"process_exited":true},"_t":12.579}
# t=12.88
{"_meta":{"exit_code":1},"_t":12.881}
# t=13.97
{"_meta":{"after_the_exit":{"lines_after_stdin_was_closed":[],"sleep_139_alive":false,"sleep_67_alive":false}},"_t":13.972}
```

All ten recordings (the time from stdin close to the exit is the difference of the two notes' stamps):

| variant | what the host did | lines after stdin was closed | exit after stdin close (s), run 1 / run 2 | exit code | `sleep` processes left |
|---|---|---|---|---|---|
| `sequence` | a turn runs (`echo go; sleep 67`): `interrupt`, the stopped turn's `result`, `stop_task` on the helper, stdin closed | none | 0.73 / 0.67 | 1 | none |
| `idle-interrupt` | no turn runs: `interrupt`, `stop_task` on the helper, stdin closed | none | 0.53 / 1.10 | 0 | none |
| `idle-close-now` | no turn runs: `stop_task` on the helper, stdin closed right after its answer | `assistant`, `assistant`, `result/success` | 1.60 / 1.73 | 0 | none |
| `idle-interrupt-again` | no turn runs: `stop_task` on the helper, wait for the `system/init` of the self-started turn, `interrupt`, that turn's `result`, stdin closed | none | 0.67 / 0.78 | 1 | none |
| `idle-outwait` | no turn runs: `stop_task` on the helper, the self-started turn's `result`, stdin closed | none | 1.10 / 0.56 | 0 | none |

What the adapter can rely on (n=2 per variant):
- With a turn running (`sequence`), the order of the question needs no extra step: the `interrupt` ends the turn AND the
  background helper (section 1), the stopped turn's `result` came 0.032, 0.023 s after the request, `stop_task` on the helper's
  id then answers `{}` and writes nothing, no turn starts by itself, and after stdin close the process was gone in
  0.73 / 0.67 s with exit code 1 (the last `result` was the interrupted one). Nothing was left running.
- With no turn running, an `interrupt` does the same to the helper (`idle-interrupt`): helper ended, no turn by itself,
  exit 0.53 / 1.10 s after stdin close, exit code 0.
- `stop_task` on the helper with NO interrupt before it starts a turn by itself (part 3, section 1): its
  `system/init` came 0.085, 0.022, 0.058, 0.084 s after the request in the four recordings that waited for it (four values are no upper
  bound: a host cannot tell "no turn will come" from "it has not come yet" by waiting a moment). What the host does
  next decides the rest:
  - stdin closed at once (`idle-close-now`; 0.161, 0.302 s after the request, when the turn's `system/init` had come): the
    process does NOT exit before that turn is over. The turn runs to its end after stdin was closed (an answer, a `result` with
    `origin.kind:"task-notification"`), and only then the process exits: 1.60 / 1.73 s after stdin close, exit code
    0. So closing stdin does not stop the self-started turn; the host pays for it and waits for it.
  - `interrupt` when that turn's `system/init` came (`idle-interrupt-again`): the answer, a `user` line
    `[Request interrupted by user]`, the turn's `result` (`error_during_execution`, `terminal_reason:"aborted_streaming"`);
    exit 0.67 / 0.78 s after stdin close, exit code 1.
  - read to the turn's `result` (`idle-outwait`): exit 1.10 / 0.56 s after stdin close, exit code 0.
- In all ten recordings the scenario's `sleep` processes ran before the stop and none was alive after the exit;
  nothing had to be signalled. The task lines come a moment before the helper's process is dead: in 1 of the 4
  recordings with an interrupt the `sleep 139` was still seen when the driver looked right after the answer, and it
  was gone at the next look.
- For a shutdown: send `interrupt` first, also when no turn runs. It ends every helper and starts no turn. Then
  `stop_task` for the background COMMANDS (they survive an interrupt: part 3, section 4), then close stdin. A
  `stop_task` on a helper is not needed after an interrupt; without an interrupt it costs one more turn.

### `foreground-stop`: `stop_task` on a helper whose `Agent` call still waits

One turn: a subagent with `run_in_background:false` (its Bash call: `echo begin; sleep 139; echo end`). 3 s into that call the host sends `stop_task` with the helper's task id. The task lines before it, then every line from the driver's note before it to the note after it (first recording):

```json
# t=1.25
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run sleep echo command","subagent_type":"general-purpose","is_backgrounded":false,"spawn_depth":1,"task_type":"local_agent","prompt":"Run the Bash command `echo begin; sleep 139; echo end` exactly as written, then report its output verbatim. Do not run any other commands."}
# t=6.86
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","owned_by_subagent":true,"tool_use_id":"toolu-2","description":"Print begin, sleep 139 seconds, print end","is_backgrounded":false,"parent_task_id":"tid-1","task_type":"local_bash"}
# t=7.08
{"_meta":{"before_stop_task":{"list":[],"helper_is_backgrounded":false,"sleep_139_alive":true}},"_t":7.075}
# t=7.08
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":7.076}
# t=7.08
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"killed","end_time":1795461082056}}
# t=7.08
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r6-shutdown-helper-foreground-stop-1/sess-1/tasks/tid-1.output","summary":"Run sleep echo command"}
# t=7.08
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{}}}
# t=7.08
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Print begin, sleep 139 seconds, print end"}
# t=7.08
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=7.08
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"[Request interrupted by user for tool use]","is_error":true,"tool_use_id":"toolu-1"}]},"…":"…"}
# t=8.26
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"The subagent call was interrupted before it returned, so I don't have any output from `echo begin; sleep 139; echo end`. I haven't retried it, since you stopped…"}],"…":"…"},"…":"…"}
# t=8.26
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"The subagent call was interrupted before it returned, so I don't have any output from `echo begin; sleep 139; echo end`. I haven't retried it, since you stopped…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=11.54
{"_meta":{"after_stop_task":{"turn_result_within_30s":true,"lines":["system/task_updated \"killed\"","system/task_notification \"stopped\"","control_response success","system/task_notification \"stopped\"","user (helper)","user","assistant","assistant","result/success"],"sleep_139_alive":false,"list":[],"results":1},"seconds_from_the_stop_task_request":{"answer":0.003,"task_updated":0.003,"task_notification":0.003,"result":1.251}},"_t":11.54}
```

What the adapter can rely on (n=2, the two recordings agree line for line up to the model's answer):
- `stop_task {"task_id": <the helper's id>}` works on a FOREGROUND helper too: `task_updated`
  (`patch.status:"killed"`), `task_notification` (`"status":"stopped"`), then the answer `{}` (0.003, 0.015 s after the
  request). No `background_tasks_changed` line (it was never listed).
- The helper's running Bash call ends with it: its own `task_notification` (`stopped`, `"output_file":""`: it was a
  task, see section 1) and one `user` line with `parent_tool_use_id` that rejects the call. Its `sleep 139` was gone
  when the host looked after the turn.
- The `Agent` call of the main thread RETURNS: a `user` line whose tool result is the error text
  `[Request interrupted by user for tool use]`, with `tool_result_meta` `"non_execution_kind":"interrupted"`.
- The TURN GOES ON: the model gets that result, answers, and the turn ends with a normal `result` (`success`). The
  stop of a foreground helper does not stop the turn, and no extra turn starts (the turn that waited is the one that
  reports it).
- So a Stop button for a foreground helper can send the same request as for a background one.

## 3. A helper that fails by itself; a background command killed by a signal from outside

Question: the status word of the `task_notification` of a helper that fails by itself; the word and the list line when a background command is killed by a signal.

Fixtures: `r6-fails-helper-background`, `r6-fails-helper-foreground` (a custom agent whose model name does not exist: its first model request is refused; flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Agent --agents '{"no-model": {"description": "A helper for tests.", "prompt": "You run the one command you are given and report its output.", "model": "claude-no-such-model-0"}}'`), `r6-fails-signal-term`, `r6-fails-signal-kill`, `r6-fails-signal-child-kill` (flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`).

### The helper

Started in the background. Task lines, the helper's own lines, results (every line of the kind up to the end of the turn the CLI starts by itself):

```json
# t=1.51
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_agent","subagent_type":"no-model","description":"Run echo hi"}]}
# t=1.51
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run echo hi","subagent_type":"no-model","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent","prompt":"Run the Bash command `echo hi` and report its output."}
# t=1.58
{"_stderr":"[claude-code:unrecognized_model] {\"model\":\"claude-no-such-model-0\",\"query_source\":\"agent:custom:no-model\"}","_t":1.577}
# t=1.96
{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"There's an issue with the selected model (claude-no-such-model-0). It may not exist or you may not have access to it. Run --model to pick a different model."}],"…":"…"},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=1.96
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"failed","end_time":1795459336915,"error":"Agent terminated early due to an API error: There's an issue with the selected model (claude-no-such-model-0). It may not exist or you may not have access to it. Run --model to pick a different model. (error type model_not_found, HTTP 404, request id apireq-2, model sent to the API: claude-no-such-model-0)"}}
# t=1.96
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"failed","output_file":"/tmp/claude-tmp/-tmp-work-r6-fails-helper-background/sess-1/tasks/tid-1.output","summary":"Agent terminated early due to an API error: There's an issue with the selected model (claude-no-such-model-0). It may not exist or you may not have access to it. Run --model to pick a different model. (error type model_not_found, HTTP 404, request id apireq-2, model sent to the API: claude-no-such-model-0)"}
# t=1.96
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=1.96
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"started","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=3.88
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The background subagent failed before it ran `echo hi`. The API rejected its model with `model_not_found` (the model ID sent was `claude-no-such-model-0`), so there is no output to report.\n\nI can run …","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
```

Started in the foreground (`run_in_background:false`): the same lines, and the result of the `Agent` call:

```json
# t=1.58
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Run echo hi via no-model","subagent_type":"no-model","is_backgrounded":false,"spawn_depth":1,"task_type":"local_agent","prompt":"Run the Bash command `echo hi` and report its output."}
# t=1.58
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Run the Bash command `echo hi` and report its output."}]},"parent_tool_use_id":"toolu-1","agent_id":"tid-1","…":"…"}
# t=1.66
{"_stderr":"[claude-code:unrecognized_model] {\"model\":\"claude-no-such-model-0\",\"query_source\":\"agent:custom:no-model\"}","_t":1.657}
# t=1.66
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"failed","end_time":1795459350572,"error":"Agent terminated early due to an API error: There's an issue with the selected model (claude-no-such-model-0). It may not exist or you may not have access to it. Run --model to pick a different model. (error type model_not_found, HTTP 404, request id apireq-2, model sent to the API: claude-no-such-model-0)"}}
# t=1.66
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"failed","output_file":"/tmp/claude-tmp/-tmp-work-r6-fails-helper-foreground/sess-1/tasks/tid-1.output","summary":"Agent terminated early due to an API error: There's an issue with the selected model (claude-no-such-model-0). It may not exist or you may not have access to it. Run --model to pick a different model. (error type model_not_found, HTTP 404, request id apireq-2, model sent to the API: claude-no-such-model-0)"}
# t=1.91
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Agent terminated early due to an API error: There's an issue with the selected model (claude-no-such-model-0). It may not exist or you may not have access to it. Run --model to pick a different model. (error type model_not_found, HTTP 404, request id apireq-2, model sent to the API: claude-no-such-model-0)","is_error":true,"tool_use_id":"toolu-1"}]},"…":"…"}
# t=2.65
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"The subagent failed before running anything: the API rejected its model (`claude-no-such-model-0`, HTTP 404, model_not_found), so the `echo hi` command never ran. I'm not retrying, as you asked.","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
```

### The command

Turn 1 starts `echo begin; sleep 131; echo end` with `run_in_background:true`. Two processes then have the command in their command line: the shell that the CLI started for it, and the `sleep` (a child of that shell). The driver sends one signal. The driver's notes and the task lines after it, for SIGTERM to the shell:

```json
# t=6.39
{"_meta":{"before_the_signal":{"list":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_bash","description":"Run delayed echo sequence in background"}],"processes_with_the_command":2,"shells_started_by_the_cli":1,"their_sleep_children":1}},"_t":6.387}
# t=6.40
{"_meta":{"signal_sent":{"signal":"SIGTERM","to":"the shell that runs the background command","delivered":true}},"_t":6.397}
# t=6.40
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"failed","end_time":1795459394931}}
# t=6.40
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"failed","output_file":"/tmp/claude-tmp/-tmp-work-r6-fails-signal-term/sess-1/tasks/tid-1.output","summary":"Background command \"Run delayed echo sequence in background\" failed with exit code 144"}
# t=6.40
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=8.56
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The background command finished with status **failed** (exit code 144). I haven't read its output file, since you asked me not to check it. I can read it now or…","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=9.87
{"_meta":{"after_the_signal":{"task_notification_status":"failed","lines":["system/task_updated \"failed\"","system/task_notification \"failed\"","system/background_tasks_changed 0 listed","system/init","assistant","assistant","result/success"],"list":[],"shell_alive":false,"sleep_alive":true},"seconds_from_the_signal":{"task_updated":0.016,"task_notification":0.016,"list_line":0.016}},"_t":9.872}
# t=9.99
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":9.989}
# t=10.90
{"_meta":{"exit_code":0},"_t":10.9}
```

| signal | sent to | `task_updated` `patch.status` | `task_notification` `status` | `summary` | list after | the shell | the `sleep` |
|---|---|---|---|---|---|---|---|
| SIGTERM | the shell that runs the background command | `failed` | `failed` | `Background command "Run delayed echo sequence in background" failed with exit code 144` | `[]` | gone | alive |
| SIGKILL | the shell that runs the background command | `failed` | `failed` | `Background command "Run delayed echo sequence in background" failed with exit code 1` | `[]` | gone | alive |
| SIGKILL | the `sleep` process (a child of the command's shell) | `completed` | `completed` | `Background command "Run delayed echo sequence in background" completed (exit code 0)` | `[]` | gone | gone |

What the adapter can rely on (n=1 per row; the two helper runs agree on the word):
- A helper that fails by itself: `task_updated` with `patch.status:"failed"` and a `patch.error` text
  (`Agent terminated early due to an API error: …`), then `task_notification` with `"status":"failed"` and the same
  text as `summary`. In the background: then `background_tasks_changed` with `[]`, and the CLI starts a turn by
  itself. In the foreground: the `Agent` call returns that text as an error tool result and the turn goes on.
- So the status words of a helper are the ones of a command: `completed`, `failed`, `stopped`
  (`task_updated.patch.status`: `completed`, `failed`, `killed`). `patch.error` is new: only the failed helper has it.
- The helper wrote one line of its own in each run: in the background run an `assistant` line with
  `"model":"<synthetic>"` and the error sentence as text, in the foreground run the `user` line with its prompt (the
  error text is then only in the task lines and in the tool result). The CLI's stderr has
  `[claude-code:unrecognized_model] …` once.
- A background command whose SHELL is killed from outside (SIGTERM or SIGKILL) ends as `failed`, like a command that
  exits with an error. There is no word for "killed by a signal". The summary says `failed with exit code 144` after
  SIGTERM and `failed with exit code 1` after SIGKILL. The lines came 0.016, 0.040, 0.022 s after the signal.
- The list line after it is `[]`, and the CLI starts a turn by itself, as after any end of a background command.
- THE LIST SAYS NOTHING ABOUT WHAT THE COMMAND LEFT BEHIND: in both shell runs the `sleep 131` child was still alive
  after the list was empty (the driver killed it at the end). "Not listed" means "the CLI no longer tracks it", not
  "none of its processes run".
- SIGKILL to the `sleep` child alone: the shell goes on (`echo end`), the command ends as `completed`
  (`completed (exit code 0)`): the death of a child is not visible at all.

## 4. `apply_flag_settings` with `effortLevel:null` as the FIRST such request on a `--effort high` process

Question: what does `get_settings` say after it? (Part 3, section 7, sent `null` after an `effortLevel:"low"`.)

Fixtures: `r6-effort-null-first-fresh` (nothing before the `null` but `initialize` and `get_settings`), `r6-effort-null-first-after-turn` (one turn before it). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --effort high`.

`fresh`: every control request after `initialize`, with its answer:

```json
{"type":"control_request","request_id":"host-2","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"effective":{},"sources":[],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"high","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-3","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":null}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3"}}
{"type":"control_request","request_id":"host-4","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"effective":{},"sources":[],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-5","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-5","response":{"effective":{},"sources":[],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-6","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"high"}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-6"}}
{"type":"control_request","request_id":"host-7","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-7","response":{"effective":{"effortLevel":"high"},"sources":[{"source":"flagSettings","settings":{"effortLevel":"high"}}],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"high","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
```

The driver's notes of both runs (`get_settings` read after each step):

| run | step | `applied.effort` | `effective.effortLevel` | sources that name `effortLevel` |
|---|---|---|---|---|
| `fresh` | as started | `high` | no such key | none |
| `fresh` | after effortLevel null | `medium` | no such key | none |
| `fresh` | after the next turn | `medium` | no such key | none |
| `fresh` | after effortLevel high | `high` | `high` | `flagSettings` |
| `after-turn` | as started | `high` | no such key | none |
| `after-turn` | after a turn | `high` | no such key | none |
| `after-turn` | after effortLevel null | `medium` | no such key | none |
| `after-turn` | after the next turn | `medium` | no such key | none |
| `after-turn` | after effortLevel high | `high` | `high` | `flagSettings` |

What the adapter can rely on (n=2 processes):
- `apply_flag_settings {"settings":{"effortLevel":null}}` as the FIRST such request on a process started with
  `--effort high` takes the start flag's level away: `applied.effort` goes from `high` to `medium` at once, before any
  turn and after one, and stays `medium` through the next turn. The answer is `{"subtype":"success"}` with no body.
- So `null` means "the default" (`medium`), never "back to the start flag", whether or not a level was applied before
  (part 3 had it after `low`). A live request can take an effort level away from a process that was started with one.
- `effortLevel:"high"` after it gives `high` again, now as a flag setting (`effective.effortLevel:"high"`,
  a `flagSettings` source); the start flag's `high` had neither.
- Still not visible in any line: the effort a model request is sent with (part 3, section 7).

## 5. `set_model` while an answer streams

Question: `set_model` sent while an answer streams and no tool call runs: the `message.model` of the stream's remaining lines, of the next turn, and the `modelUsage` of the result.

Fixtures: `r6-model-while-streaming-1`, `r6-model-while-streaming-2` (the same scenario twice). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

Turn 1 (on haiku) asks for the numbers from 1 to 400. When the first text delta of the answer has come the host sends `set_model sonnet`. Turn 2 is one tiny turn; then `set_model haiku` and turn 3. Requests, init lines, assistant lines and the driver's note per turn (first recording):

```json
# t=0.34
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Write the numbers from 1 to 400, one per line, and nothing else."},"parent_tool_use_id":null},"_t":0.342}
# t=0.34
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=0.94
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"set_model","model":"sonnet"}},"_t":0.942}
# t=0.94
{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to `sonnet (claude-sonnet-5-5)`</local-command-stdout>"},"isReplay":true,"…":"…"}
# t=0.94
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"model":"claude-sonnet-5-5"}}}
# t=3.04
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n1…"}],"…":"…"},"…":"…"}
# t=3.20
{"_meta":{"turn":"turn 1","init_model":["claude-haiku-5-5"],"result_modelUsage_keys":[["claude-haiku-5-5"]],"result_model_usage_since_last_result_keys":[["claude-haiku-5-5"]],"before_the_set_model_answer":{"stream_message_start_model":["claude-haiku-5-5"],"assistant_message_model":[],"text_deltas":1},"after_the_set_model_answer":{"stream_message_start_model":[],"assistant_message_model":["claude-haiku-5-5"],"text_deltas":100}},"_t":3.198}
# t=3.20
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: two"},"parent_tool_use_id":null},"_t":3.198}
# t=3.20
{"type":"system","subtype":"init","model":"claude-sonnet-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=3.91
{"type":"assistant","message":{"model":"claude-sonnet-5-5","content":[{"type":"text","text":"two"}],"…":"…"},"…":"…"}
# t=4.01
{"_meta":{"turn":"turn 2","init_model":["claude-sonnet-5-5"],"result_modelUsage_keys":[["claude-haiku-5-5","claude-sonnet-5-5"]],"result_model_usage_since_last_result_keys":[["claude-sonnet-5-5"]],"stream_message_start_model":["claude-sonnet-5-5"],"assistant_message_model":["claude-sonnet-5-5"],"text_deltas":1},"_t":4.011}
# t=4.01
{"_sent":{"type":"control_request","request_id":"host-4","request":{"subtype":"set_model","model":"haiku"}},"_t":4.011}
# t=4.01
{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to `haiku (claude-haiku-5-5)`</local-command-stdout>"},"isReplay":true,"…":"…"}
# t=4.01
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"model":"claude-haiku-5-5"}}}
# t=4.01
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: three"},"parent_tool_use_id":null},"_t":4.013}
# t=4.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=4.71
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"three"}],"…":"…"},"…":"…"}
# t=4.83
{"_meta":{"turn":"turn 3","init_model":["claude-haiku-5-5"],"result_modelUsage_keys":[["claude-haiku-5-5","claude-sonnet-5-5"]],"result_model_usage_since_last_result_keys":[["claude-haiku-5-5"]],"stream_message_start_model":["claude-haiku-5-5"],"assistant_message_model":["claude-haiku-5-5"],"text_deltas":1},"_t":4.829}
```

What the adapter can rely on (n=2, the two recordings agree):
- A `set_model` that arrives while an answer streams does not touch that answer. The request that is in flight goes
  on: 100 and 100 text deltas came after the answer to `set_model`, there was no second `message_start`, and the
  `assistant` line that closes the answer says `claude-haiku-5-5`. The turn's `result.modelUsage` has haiku only.
- `set_model` itself is answered at once (`{"model":"claude-sonnet-5-5"}`), with the `isReplay` `user` line before it
  (as in part 3), and `get_settings.applied.model` says sonnet while the haiku answer still streams.
- The NEXT turn is on the new model: `system/init.model`, `message.model` and a new key in `result.modelUsage`
  (`model_usage_since_last_result` has sonnet alone).
- Together with part 3, section 8 (a switch during a TOOL CALL shows on the next assistant line of the same turn):
  the switch takes effect at the next model REQUEST. A line on the old model after the switch is the request that
  was already in flight, and at most that one request.

## 6. Sandbox network: a host that was DENIED once

Question: after the network request for a host was denied, is the next command to the same host asked about again in the same process?

Fixtures: `r6-sandbox-network-deny-1`, `r6-sandbox-network-deny-2` (the same scenario twice). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true}}'`. The capture folder was outside the CLI's own temp folder.

Five turns, one command each. Turns 1 to 4: `curl -sI https://example.com`; the request for the host is denied in turn 1, denied again in turn 2, allowed in turn 3. Turn 5: `curl -sI https://example.org` (allowed). Every `can_use_tool` request with its answer, the first line of each tool result, and the driver's note per turn (first recording):

```json
# t=3.31
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"SandboxNetworkAccess","display_name":"SandboxNetworkAccess","input":{"host":"example.com"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"WebFetch","ruleContent":"domain:example.com"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"uuid-17","description":"Allow network connection to example.com?"}}
# t=3.31
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"deny","message":"The user denied this action.","interrupt":false}}},"_t":3.311}
# t=3.31
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Exit code 56\nHTTP/1.1 403 user denied\r\nContent-Type: text/plain\r\nX-Proxy-Error: blocked-by-allowlist\r\n\r\n\n<sandbox_violations>\ndeny network-outbound ex…","is_error":true,"tool_use_id":"toolu-1"}]},"…":"…"}
# t=5.54
{"_meta":{"turn":1,"command":"curl -sI https://example.com","can_use_tool_requests":[["SandboxNetworkAccess","example.com",null]],"answers":["deny"],"tool_calls_of_the_turn":1},"_t":5.542}
# t=7.17
{"type":"control_request","request_id":"req-2","request":{"subtype":"can_use_tool","tool_name":"SandboxNetworkAccess","display_name":"SandboxNetworkAccess","input":{"host":"example.com"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"WebFetch","ruleContent":"domain:example.com"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"uuid-92","description":"Allow network connection to example.com?"}}
# t=7.17
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-2","response":{"behavior":"deny","message":"The user denied this action.","interrupt":false}}},"_t":7.17}
# t=7.17
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Exit code 56\nHTTP/1.1 403 user denied\r\nContent-Type: text/plain\r\nX-Proxy-Error: blocked-by-allowlist\r\n\r\n\n<sandbox_violations>\ndeny network-outbound ex…","is_error":true,"tool_use_id":"toolu-2"}]},"…":"…"}
# t=8.19
{"_meta":{"turn":2,"command":"curl -sI https://example.com","can_use_tool_requests":[["SandboxNetworkAccess","example.com",null]],"answers":["deny"],"tool_calls_of_the_turn":1},"_t":8.187}
# t=11.24
{"type":"control_request","request_id":"req-3","request":{"subtype":"can_use_tool","tool_name":"SandboxNetworkAccess","display_name":"SandboxNetworkAccess","input":{"host":"example.com"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"WebFetch","ruleContent":"domain:example.com"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"uuid-146","description":"Allow network connection to example.com?"}}
# t=11.24
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-3","response":{"behavior":"allow","updatedInput":{"host":"example.com"}}}},"_t":11.244}
# t=11.78
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-3","type":"tool_result","content":"HTTP/1.1 200 Connection Established\r\n\r\nHTTP/2 200 \r\ndate: Mon, 23 Nov 2026 18:44:19 GMT\r\ncontent-type: text/html; charset=utf-8\r\nserver: cloudflare\r\nl…","is_error":false}]},"…":"…"}
# t=13.11
{"_meta":{"turn":3,"command":"curl -sI https://example.com","can_use_tool_requests":[["SandboxNetworkAccess","example.com",null]],"answers":["allow"],"tool_calls_of_the_turn":1},"_t":13.113}
# t=15.63
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-4","type":"tool_result","content":"HTTP/1.1 200 Connection Established\r\n\r\nHTTP/2 200 \r\ndate: Mon, 23 Nov 2026 18:44:23 GMT\r\ncontent-type: text/html; charset=utf-8\r\nserver: cloudflare\r\nl…","is_error":false}]},"…":"…"}
# t=16.52
{"_meta":{"turn":4,"command":"curl -sI https://example.com","can_use_tool_requests":[],"answers":[],"tool_calls_of_the_turn":1},"_t":16.52}
# t=19.52
{"type":"control_request","request_id":"req-4","request":{"subtype":"can_use_tool","tool_name":"SandboxNetworkAccess","display_name":"SandboxNetworkAccess","input":{"host":"example.org"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"WebFetch","ruleContent":"domain:example.org"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"uuid-226","description":"Allow network connection to example.org?"}}
# t=19.52
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-4","response":{"behavior":"allow","updatedInput":{"host":"example.org"}}}},"_t":19.521}
# t=20.37
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-5","type":"tool_result","content":"HTTP/1.1 200 Connection Established\r\n\r\nHTTP/2 200 \r\ndate: Mon, 23 Nov 2026 18:44:28 GMT\r\ncontent-type: text/html; charset=utf-8\r\nserver: cloudflare\r\nl…","is_error":false}]},"…":"…"}
# t=21.28
{"_meta":{"turn":5,"command":"curl -sI https://example.org","can_use_tool_requests":[["SandboxNetworkAccess","example.org",null]],"answers":["allow"],"tool_calls_of_the_turn":1},"_t":21.282}
```

What the adapter can rely on (n=2 processes, two "next commands" after a deny in each):
- YES, it is asked about again. A deny of the `SandboxNetworkAccess` request is for that one command: the next
  command to the same host got a new request (turn 2 after the deny of turn 1, turn 3 after the second deny; 4 of 4).
- The denied command gets a network error from the sandbox's proxy: the tool result starts with `Exit code 56` and
  `HTTP/1.1 403 user denied`, with `X-Proxy-Error: blocked-by-allowlist`.
- An allow after two denies works like any allow: the command of turn 3 ran, and turn 4 (the same host) was NOT asked
  about (part 3, section 6: a plain allow lasts for the process).
- So a deny and an allow are not symmetric: an allow is remembered for the host until the process ends, a deny is
  not remembered at all.
- A deny writes no rule: `list_permission_rules` has no rule for the host after any turn, and
  `get_sandbox_dialog.restrictions.network_allowed_domains` stays `[]`.
- The deny answer that was sent is `{"behavior":"deny","message":"The user denied this action.","interrupt":false}`.
  Not recorded: a deny with `"interrupt":true`, and a deny that carries `updatedPermissions` (a saved deny rule).

## 7. MCP: `mcp_reconnect` on a connected server; a stdio server whose process is killed

Question: is a connected server started again by `mcp_reconnect`? When the process of a stdio server is killed mid-session: does `mcp_status` change by itself, is there a line, what does the next call of its tool return?

Fixtures: `r6-mcp-live-server-status-first`, `r6-mcp-live-server-call-first`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r6-mcp-live-server-status-first.json`. The server `ping-live` (the drivers folder's `mcp_ping_server.py`) writes a line with its pid each time it starts; the driver's notes say how often it was started and which of those processes live.

`status-first`: `mcp_status`, `mcp_reconnect`, `mcp_status`, a turn that calls the tool; the driver kills the server process (SIGKILL) and reads for 8 s; `mcp_status`, a turn that calls the tool, `mcp_status`, `mcp_reconnect`, `mcp_status`, a turn. Every control request with its answer, the driver's notes, the init lines and the tool results:

```json
# t=3.02
{"_meta":{"after":"start + 3 s","server_starts_so_far":1,"server_processes_alive":[true]},"_t":3.015}
# t=3.02
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"mcp_status"}},"_t":3.016}
# t=3.02
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"mcpServers":[{"name":"ping-live","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=3.54
{"_meta":{"after_the_mcp_status_answer":true,"server_starts_so_far":1,"server_processes_alive":[true]},"_t":3.537}
# t=3.54
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"mcp_reconnect","serverName":"ping-live"}},"_t":3.537}
# t=3.54
{"type":"control_response","response":{"subtype":"success","request_id":"host-2"}}
# t=3.67
{"_meta":{"after":"mcp_reconnect on the connected server","server_starts_so_far":2,"server_processes_alive":[false,true]},"_t":3.671}
# t=3.67
{"_sent":{"type":"control_request","request_id":"host-3","request":{"subtype":"mcp_status"}},"_t":3.671}
# t=3.67
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"mcpServers":[{"name":"ping-live","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=4.20
{"_meta":{"after_the_mcp_status_answer":true,"server_starts_so_far":2,"server_processes_alive":[false,true]},"_t":4.196}
# t=4.20
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash","mcp__ping-live__ping"],"mcp_servers":[{"name":"ping-live","status":"connected","source":"dynamic"}],"…":"…"}
# t=4.96
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":[{"type":"text","text":"pong"}]}]},"…":"…"}
# t=5.65
{"_meta":{"killed":{"signal":"SIGKILL","to":"the server process (the last pid in started.txt)","delivered":true}},"_t":5.652}
# t=13.76
{"_meta":{"stdout_lines_in_the_8s_after_the_kill":[],"server_process_alive":false},"_t":13.755}
# t=13.81
{"_meta":{"after":"the server process was killed 8 s ago","server_starts_so_far":2,"server_processes_alive":[false,false]},"_t":13.809}
# t=13.81
{"_sent":{"type":"control_request","request_id":"host-4","request":{"subtype":"mcp_status"}},"_t":13.809}
# t=13.81
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"mcpServers":[{"name":"ping-live","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=14.53
{"_meta":{"after_the_mcp_status_answer":true,"server_starts_so_far":2,"server_processes_alive":[false,false]},"_t":14.53}
# t=14.53
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash","mcp__ping-live__ping"],"mcp_servers":[{"name":"ping-live","status":"connected","source":"dynamic"}],"…":"…"}
# t=15.25
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":[{"type":"text","text":"pong"}]}]},"…":"…"}
# t=16.29
{"_meta":{"after":"the turn after the kill","server_starts_so_far":3,"server_processes_alive":[false,false,true]},"_t":16.286}
# t=16.29
{"_sent":{"type":"control_request","request_id":"host-5","request":{"subtype":"mcp_status"}},"_t":16.287}
# t=16.29
{"type":"control_response","response":{"subtype":"success","request_id":"host-5","response":{"mcpServers":[{"name":"ping-live","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=17.00
{"_meta":{"after_the_mcp_status_answer":true,"server_starts_so_far":3,"server_processes_alive":[false,false,true]},"_t":16.997}
# t=17.00
{"_sent":{"type":"control_request","request_id":"host-6","request":{"subtype":"mcp_reconnect","serverName":"ping-live"}},"_t":16.997}
# t=17.00
{"type":"control_response","response":{"subtype":"success","request_id":"host-6"}}
# t=17.67
{"_meta":{"after":"mcp_reconnect after the kill","server_starts_so_far":4,"server_processes_alive":[false,false,false,true]},"_t":17.671}
# t=17.67
{"_sent":{"type":"control_request","request_id":"host-7","request":{"subtype":"mcp_status"}},"_t":17.671}
# t=17.67
{"type":"control_response","response":{"subtype":"success","request_id":"host-7","response":{"mcpServers":[{"name":"ping-live","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=18.59
{"_meta":{"after_the_mcp_status_answer":true,"server_starts_so_far":4,"server_processes_alive":[false,false,false,true]},"_t":18.585}
# t=18.59
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash","mcp__ping-live__ping"],"mcp_servers":[{"name":"ping-live","status":"connected","source":"dynamic"}],"…":"…"}
# t=19.50
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-3","type":"tool_result","content":[{"type":"text","text":"pong"}]}]},"…":"…"}
# t=20.22
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":20.219}
# t=21.09
{"_meta":{"exit_code":0},"_t":21.089}
```

`call-first` is the same without the `mcp_status` between the kill and the turn. From the kill to the `mcp_status` after the turn:

```json
# t=5.42
{"_meta":{"killed":{"signal":"SIGKILL","to":"the server process (the last pid in started.txt)","delivered":true}},"_t":5.415}
# t=13.72
{"_meta":{"stdout_lines_in_the_8s_after_the_kill":[],"server_process_alive":false},"_t":13.725}
# t=13.72
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash","mcp__ping-live__ping"],"mcp_servers":[{"name":"ping-live","status":"connected","source":"dynamic"}],"…":"…"}
# t=14.33
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":[{"type":"text","text":"pong"}]}]},"…":"…"}
# t=15.22
{"_meta":{"after":"the turn after the kill","server_starts_so_far":3,"server_processes_alive":[false,false,true]},"_t":15.218}
# t=15.22
{"_sent":{"type":"control_request","request_id":"host-4","request":{"subtype":"mcp_status"}},"_t":15.218}
# t=15.22
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"mcpServers":[{"name":"ping-live","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=15.87
{"_meta":{"after_the_mcp_status_answer":true,"server_starts_so_far":3,"server_processes_alive":[false,false,true]},"_t":15.869}
# t=15.87
{"_sent":{"type":"control_request","request_id":"host-5","request":{"subtype":"mcp_reconnect","serverName":"ping-live"}},"_t":15.869}
# t=16.23
{"type":"control_response","response":{"subtype":"success","request_id":"host-5"}}
```

What the adapter can rely on (n=2):
- `mcp_reconnect {"serverName"}` on a server that is `connected` DOES start it again: the answer is
  `{"subtype":"success"}` with no body, the old process was gone and a new one ran (start counter 1 -> 2), and
  `mcp_status` says `connected`. So Reconnect is safe to offer for a connected server; it is a restart.
- A stdio server whose process is killed (SIGKILL from outside): NO line by itself in the 8 s the host read, and
  `mcp_status` 8 s after the kill still says `"status":"connected"` with `serverInfo` and `tools`, while no process of
  the server lives. The request does not start one either (the counter stays at 2 after the answer). So
  `mcp_status` does not show a dead stdio server.
- The next turn that calls its tool WORKS: `system/init` says `connected`, the tool is in `init.tools`, the request
  comes as usual, and the tool result is `pong`. The CLI started the server again by itself during that turn (counter
  2 -> 3), with or without an `mcp_status` before it. Nothing in the stream says that it did.
- So a host cannot see from the stream that a stdio server died, and for a server that can be started again it does
  not need to: the call heals it. What a call returns when the server CANNOT be started again is not recorded here
  (a server that fails at start is part 3, section 10: `failed`, `Connection closed`).
- `mcp_reconnect` after that is one more restart (counter 3 -> 4).

## 8. A project `.mcp.json` server with the app's flags and the REAL login

Question: one tool call each with `enableAllProjectMcpServers:false`, with an `enabledMcpjsonServers` list that does not name the server, and with `disabledMcpjsonServers` naming it (all through `--settings`); and with `--setting-sources user` and no `--strict-mcp-config`: is the server loaded, can its tool be called? Added after a review: the same with `--setting-sources user,local` when the folder's own `.claude/settings.local.json` turns the project's servers on.

Fixtures: `r6-project-mcp-enable-all-false`, `-enabled-other`, `-disabled` (real login; `--permission-mode manual --setting-sources user,project --tools Bash --settings '{"enableAllProjectMcpServers": false, "disableAllHooks": true, "disableClaudeAiConnectors": true}'`), `-user-only` (real login; `--permission-mode manual --setting-sources user --tools Bash --settings '{"disableAllHooks": true, "disableClaudeAiConnectors": true}'`), `-user-only-home` and `-user-local-home` (a throw-away HOME, no login), `-user-local-file` (real login; `--permission-mode manual --setting-sources user,local --tools Bash --settings '{"disableAllHooks": true, "disableClaudeAiConnectors": true}'`), `-user-local-file-home-1` and `-2` (a throw-away HOME, no login), and the two checks `-hooks-on`, `-hooks-off`. No run has `--strict-mcp-config`. The work folder's `.mcp.json` defines `ping-project`.

`enable-all-false`: the driver's notes, the init line, the request and the tool result:

```json
# t=0.00
{"_meta":{"env_set":{"ENABLE_CLAUDEAI_MCP_SERVERS":"false"}},"_t":0.0}
# t=3.01
{"_meta":{"before_first_message":true,"server_started":true},"_t":3.012}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","mcp__ping-project__ping"],"mcp_servers":[{"name":"ping-project","status":"connected","source":"project"}],"…":"…"}
# t=3.81
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"mcp__ping-project__ping","mcp_server":{"name":"ping-project","source":"project"},"display_name":"Ping","input":{},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"mcp__ping-project__ping"}],"behavior":"allow","destination":"localSettings"}],"server_policy_ask_only":true,"tool_use_id":"toolu-1"}}
# t=4.05
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":[{"type":"text","text":"pong"}]}]},"…":"…"}
# t=4.84
{"_meta":{"server_starts":1,"init_entry_for_this_server":[{"name":"ping-project","status":"connected","source":"project"}],"tool_in_init":true,"servers_of_the_users_own_setup_in_init":true,"hook_lines_in_the_stream":0,"hook_file_lines":0,"can_use_tool_requests":[["mcp__ping-project__ping",{"name":"ping-project","source":"project"},"allowed"]],"result":"pong"},"_t":4.84}
```

`user-only` (the user's own settings are the only source), the same lines:

```json
# t=0.00
{"_meta":{"env_set":{"ENABLE_CLAUDEAI_MCP_SERVERS":"false"}},"_t":0.0}
# t=3.01
{"_meta":{"before_first_message":true,"server_started":false},"_t":3.014}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=4.63
{"_meta":{"server_starts":0,"init_entry_for_this_server":[],"tool_in_init":false,"servers_of_the_users_own_setup_in_init":true,"hook_lines_in_the_stream":0,"hook_file_lines":0,"can_use_tool_requests":[],"result":"none"},"_t":4.628}
```

`user-local-file-home-1` (`--setting-sources user,local`, no login): the lines of the `SessionStart` hook, the init line and the driver's notes. One note has the folder's `.claude/settings.local.json` (the driver wrote the file before the start and the note 3 s after it):

```json
# t=0.00
{"_meta":{"env_set":{"HOME":"/tmp/work/r6-project-mcp-user-local-file-home-1-home"}},"_t":0.0}
# t=0.00
{"type":"system","subtype":"hook_started","hook_id":"hook-1","hook_name":"SessionStart:startup","hook_event":"SessionStart"}
# t=0.00
{"type":"system","subtype":"hook_response","hook_id":"hook-1","hook_name":"SessionStart:startup","hook_event":"SessionStart","output":"","stdout":"","stderr":"","exit_code":0,"outcome":"success"}
# t=3.01
{"_meta":{"settings_local_json_of_the_work_folder":{"enableAllProjectMcpServers":true,"enabledMcpjsonServers":["ping-project"],"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo ran >> /tmp/work/r6-project-mcp-user-local-file-home-1/hook-ran.txt"}]}],"UserPromptSubmit":[{"hooks":[{"type":"command","command":"echo ran >> /tmp/work/r6-project-mcp-user-local-file-home-1/hook-ran.txt"}]}]}}},"_t":3.013}
# t=3.01
{"_meta":{"before_first_message":true,"server_started":false},"_t":3.013}
# t=3.73
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=3.85
{"_meta":{"server_starts":0,"init_entry_for_this_server":[],"tool_in_init":false,"servers_of_the_users_own_setup_in_init":false,"hook_lines_in_the_stream":2,"hook_file_lines":2,"can_use_tool_requests":[],"result":"Not logged in · Please run /login"},"_t":3.849}
```

| run | `--setting-sources` | the setting in `--settings` | `.claude/settings.local.json` in the folder | login | server started | init entry | the tool call | the user's own servers in init |
|---|---|---|---|---|---|---|---|---|
| `enable-all-false` | `user,project` | `{"enableAllProjectMcpServers":false}` | none | real | yes | `{"name":"ping-project","status":"connected","source":"project"}` | called: `pong` | yes |
| `enabled-other` | `user,project` | `{"enabledMcpjsonServers":["another-server"]}` | none | real | yes | `{"name":"ping-project","status":"connected","source":"project"}` | called: `pong` | yes |
| `disabled` | `user,project` | `{"disabledMcpjsonServers":["ping-project"]}` | none | real | no | none | the model answered `none` | yes |
| `user-only` | `user` | none | none | real | no | none | the model answered `none` | yes |
| `user-only-home` | `user` | none | none | none (throw-away HOME) | no | none | not possible (no login) | no |
| `user-local-home` | `user,local` | none | none | none (throw-away HOME) | no | none | not possible (no login) | no |
| `user-local-file` | `user,local` | none | turns the project's servers on | real | no | none | the model answered `none` | yes |
| `user-local-file-home-1` | `user,local` | none | turns the project's servers on | none (throw-away HOME) | no | none | not possible (no login) | no |
| `user-local-file-home-2` | `user,local` | none | turns the project's servers on | none (throw-away HOME) | no | none | not possible (no login) | no |

What the adapter can rely on (n=1 per row; the two `user-local-file-home` rows are the same scenario twice):
- With the real login the three keys do what part 3 (section 12) saw without a login. `enableAllProjectMcpServers:false`
  and an `enabledMcpjsonServers` list that does not name the server do NOT keep it out: it was started, is `connected`
  with `"source":"project"`, and its tool was called (`pong`). `disabledMcpjsonServers` naming it DOES keep it out:
  not started, not in `init`, the model had no such tool.
- `--setting-sources user` with NO `--strict-mcp-config`: the project's `.mcp.json` server is NOT loaded (not started,
  not in `init.mcp_servers`, not in `init.tools`; the model answered `none`), while the servers of the user's own
  setup ARE in the init line. The same without a login, and with `--setting-sources user,local` (no login).
- `--setting-sources user,local` when the folder's `.claude/settings.local.json` turns the project's servers on
  (`"enableAllProjectMcpServers":true` and `"enabledMcpjsonServers":["ping-project"]`, the keys an earlier approval of
  a repo's servers leaves in that file): the server is still NOT loaded. Not started, not in `init.mcp_servers`, not
  in `init.tools`, in all three recordings (no login: n=2; real login: n=1, the model answered `none` and the user's
  own servers were in the init line). In the two runs without a login the CLI did read that file: the file's two
  hooks ran (2 lines in the hooks' file; in the stream the `hook_started` and `hook_response` lines of the
  `SessionStart` hook, and no line for `UserPromptSubmit`). In the logged-in run the hooks are off
  (`disableAllHooks`, as in every logged-in run), so that run has no sign of its own that the file was read; it has
  the same sources as the other two.
- So in these recordings the project's `.mcp.json` is read only when `project` is one of the setting sources; the two
  keys in a local settings file do not bring it back. In an untrusted folder (`user`, or `user,local`) the host does
  not need `--strict-mcp-config` to keep the repo's servers out, and without it the user's own servers stay.
- Weakness: one logged-in recording for `user` and one for `user,local` (n=1 each), on one machine and one CLI
  version. Not recorded: an approval of the repo's servers that is kept in the user's `~/.claude.json` (under the
  project's path) and not in the local settings file; a LOCAL-scope server (`claude mcp add` without a scope, kept
  in `~/.claude.json` under the project); `user,local` with a local file that has only one of the two keys.
- The check of this section's own safety: with a throw-away HOME whose `settings.json` has a `SessionStart` and a
  `UserPromptSubmit` hook, both ran (2 hook lines in the stream, 2 lines in the hooks' file); with
  `"disableAllHooks":true` in `--settings`, none (0 and 0). The five logged-in runs had no hook line either.

## 9. `--resume` from another folder, and of a damaged transcript

Question: the `initialize` answer, the last lines and the exit code, 3 runs each.

Fixtures: `r6-resume-source` (the session that is resumed: two turns, the answers are `apple` and `pear`), `r6-resume-other-folder-1` … `-3` (`--resume <its id>` started in another work folder, a separate git repo next to the first), `r6-resume-damaged-tail-1` … `-3` (`--resume` of a copy of its transcript file whose LAST line is cut in half), `r6-resume-damaged-mid-1` … `-3` (a copy that ends in the middle of the line that holds the second message). Flags of a resume: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --resume sess-1 --tools Bash`. The host writes `initialize` right after the spawn. A process that answers it and stays gets one message.

The first run from another folder (the body of the `initialize` answer is left out: it is the usual long one):

```json
# t=0.01
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"initialize","hooks":null}},"_t":0.012}
# t=1.29
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"…":"…"}}}
# t=2.30
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Which single words have you replied with so far in this conversation? Reply with those words only, in order, or with the word nothing."},"parent_tool_use_id":null},"_t":2.301}
# t=2.30
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=5.03
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"apple pear"}],"…":"…"},"…":"…"}
# t=5.03
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"apple pear","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=5.18
{"_meta":{"about_the_session":{"kind":"a whole session of another folder"},"initialize_answered":true,"stdout_lines_in_order":["initialize answer (success)","system/init","result/success"],"exited_by_itself":false,"a_turn_was_sent":true,"turn_result":"apple pear","result_session_id_is_the_resumed_one":true},"_t":5.177}
# t=5.18
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":5.177}
# t=6.24
{"_meta":{"exit_code":0},"_t":6.238}
# t=6.24
{"_meta":{"session_files_with_this_id":{"-tmp-work-r6-resume-source":42}},"_t":6.241}
```

| run | what was resumed | `initialize` answered | lines | the turn's answer | exit code | after the run |
|---|---|---|---|---|---|---|
| `other-folder-1` | the whole session, from another folder | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple pear` | 0 | the session's one file is still in the FIRST folder's project folder (42 lines) |
| `other-folder-2` | the whole session, from another folder | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple pear` | 0 | the session's one file is still in the FIRST folder's project folder (50 lines) |
| `other-folder-3` | the whole session, from another folder | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple pear` | 0 | the session's one file is still in the FIRST folder's project folder (58 lines) |
| `damaged-tail-1` | a copy cut in line 29 of 29 (a `cost-state` line; 284 of its 568 bytes kept); whole before the cut: 2 message lines of the user, 2 of the assistant | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple pear` | 0 | the file has 38 lines, 1 of them not JSON |
| `damaged-tail-2` | a copy cut in line 29 of 29 (a `cost-state` line; 284 of its 568 bytes kept); whole before the cut: 2 message lines of the user, 2 of the assistant | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple\npear` | 0 | the file has 38 lines, 1 of them not JSON |
| `damaged-tail-3` | a copy cut in line 29 of 29 (a `cost-state` line; 284 of its 568 bytes kept); whole before the cut: 2 message lines of the user, 2 of the assistant | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple\npear` | 0 | the file has 38 lines, 1 of them not JSON |
| `damaged-mid-1` | a copy cut in line 23 of 29 (a `user` line; 325 of its 651 bytes kept); whole before the cut: 1 message lines of the user, 1 of the assistant | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple` | 0 | the file has 33 lines, 1 of them not JSON |
| `damaged-mid-2` | a copy cut in line 23 of 29 (a `user` line; 325 of its 651 bytes kept); whole before the cut: 1 message lines of the user, 1 of the assistant | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple` | 0 | the file has 33 lines, 1 of them not JSON |
| `damaged-mid-3` | a copy cut in line 23 of 29 (a `user` line; 325 of its 651 bytes kept); whole before the cut: 1 message lines of the user, 1 of the assistant | yes | `initialize answer (success)`, `system/init`, `result/success` | `apple` | 0 | the file has 33 lines, 1 of them not JSON |

What the adapter can rely on (n=3 per row, the three runs of each row agree):
- NEITHER case is a failed start. In 9 of 9 runs the `initialize` request was answered, the process stayed, the turn
  that was sent ran, the `result.session_id` was the id that was asked for, stderr was empty and the exit code was 0.
- From ANOTHER FOLDER the session is found by its id: the turn knew both earlier answers (`apple`, `pear`). The CLI
  works in the new folder (`init.cwd` is the new folder), keeps the session id, and goes on writing the session's file in the project
  folder of the FIRST folder (`~/.claude/projects/<first folder>/<id>.jsonl`): no file with that id appears for the new
  folder. So "No conversation found" (part 3, section 9) is not what a resume from another folder gives.
- A DAMAGED transcript is read up to the damage. Last line cut in half: the conversation is whole (`apple`, `pear`).
  Cut in the line of the second message: the first turn is there (`apple`), the second is gone, and nothing says so:
  no stderr line, no error, a normal `system/init`.
- After a resume of a damaged file the damaged line stays in the file (1 line that is not JSON), and the CLI appends
  after it.
- A side finding about money: `total_cost_usd` after a resume comes from the transcript. From another folder it went
  on from the session's total (0.000355 after 0.000116). With the last line cut (a `cost-state` line) it started again
  (0.000060): a host that reads the session total after a resume cannot trust it across a damaged file.
- So for the two cases asked about there is no failure to tell apart from `SessionGone`: the start succeeds. The only
  recorded answer of the CLI to a conversation it cannot open is still the one of part 3 (unknown id).

## 10. `--max-budget-usd` on a resumed process, reached in the middle of a turn

Question: the limit is reached after a tool call of a turn on a RESUMED process: the result line, `total_cost_usd`, and a second turn after it.

Fixtures: `r6-budget-midturn-1-a`, `-1-b`, `-1-c1` … `-1-c3` and `r6-budget-midturn-2-a`, `-2-b`, `-2-c1` (the scenario twice). Run a: a new session, two turns. Run b: `--resume`, no budget, one turn (T = its cost). Runs c: `--resume` with `--max-budget-usd` = a part of T, one turn, then one more message. Flags of a run c: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --resume sess-1 --max-budget-usd 0.000225`. Every turn asks for three tool calls in a row (`echo one`, `echo two`, `echo three`) and a one-word answer.

The driver's note of every run:

```json
{"_meta":{"run":"a","budget_usd":null,"turn_ok":true,"result_subtype":"success","errors":null,"total_cost_usd":0.0014643599999999998,"cost_of_the_last_turn_alone":0.00043314000000000004,"tool_calls_that_ran_in_the_last_turn":null},"_t":12.004}
{"_meta":{"run":"b","budget_usd":null,"turn_ok":true,"result_subtype":"success","errors":null,"total_cost_usd":0.0019134999999999998,"cost_of_the_last_turn_alone":0.00044914,"tool_calls_that_ran_in_the_last_turn":3},"_t":6.408}
{"_meta":{"run":"c1","budget_usd":0.00022457,"turn_ok":true,"result_subtype":"error_max_budget_usd","errors":["Reached maximum budget ($0.000225)"],"total_cost_usd":0.00219704,"cost_of_the_last_turn_alone":0.00028354,"tool_calls_that_ran_in_the_last_turn":0,"alive_2s_after_the_limit":true,"second_turn":{"result_came":true,"result_subtype":"error_max_budget_usd","result":null,"errors":["Reached maximum budget ($0.000225)"],"total_cost_usd":0.00219704,"assistant_lines":0,"alive_after_it":true}},"_t":7.586}
{"_meta":{"run":"c2","budget_usd":0.00031439799999999997,"turn_ok":true,"result_subtype":"error_max_budget_usd","errors":["Reached maximum budget ($0.000314)"],"total_cost_usd":0.00255306,"cost_of_the_last_turn_alone":0.00035602,"tool_calls_that_ran_in_the_last_turn":0,"alive_2s_after_the_limit":true,"second_turn":{"result_came":true,"result_subtype":"error_max_budget_usd","result":null,"errors":["Reached maximum budget ($0.000314)"],"total_cost_usd":0.00255306,"assistant_lines":0,"alive_after_it":true}},"_t":7.586}
{"_meta":{"run":"c3","budget_usd":0.000381769,"turn_ok":true,"result_subtype":"error_max_budget_usd","errors":["Reached maximum budget ($0.000382)"],"total_cost_usd":0.0030055599999999996,"cost_of_the_last_turn_alone":0.0004525,"tool_calls_that_ran_in_the_last_turn":1,"alive_2s_after_the_limit":true,"second_turn":{"result_came":true,"result_subtype":"error_max_budget_usd","result":null,"errors":["Reached maximum budget ($0.000382)"],"total_cost_usd":0.0030055599999999996,"assistant_lines":0,"alive_after_it":true}},"_t":9.201}
{"_meta":{"run":"a","budget_usd":null,"turn_ok":true,"result_subtype":"success","errors":null,"total_cost_usd":0.0014643599999999998,"cost_of_the_last_turn_alone":0.00043314000000000004,"tool_calls_that_ran_in_the_last_turn":null},"_t":8.676}
{"_meta":{"run":"b","budget_usd":null,"turn_ok":true,"result_subtype":"success","errors":null,"total_cost_usd":0.0019134999999999998,"cost_of_the_last_turn_alone":0.00044914,"tool_calls_that_ran_in_the_last_turn":3},"_t":14.864}
{"_meta":{"run":"c1","budget_usd":0.00022457,"turn_ok":true,"result_subtype":"error_max_budget_usd","errors":["Reached maximum budget ($0.000225)"],"total_cost_usd":0.00227936,"cost_of_the_last_turn_alone":0.00036585999999999997,"tool_calls_that_ran_in_the_last_turn":1,"alive_2s_after_the_limit":true,"second_turn":{"result_came":true,"result_subtype":"error_max_budget_usd","result":null,"errors":["Reached maximum budget ($0.000225)"],"total_cost_usd":0.00227936,"assistant_lines":0,"alive_after_it":true}},"_t":10.063}
```

`r6-budget-midturn-1-c3`, the run whose limit came after a tool call had run: every line but the stream events:

```json
# t=0.01
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `echo one`. After you see its output, run `echo two` in a second, separate tool call. After that, run `echo three` in a third, separate tool call. Then reply with the single word: done"},"parent_tool_use_id":null},"_t":0.011}
# t=1.47
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=2.63
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"thinking","…":"…"}],"…":"…"},"…":"…"}
# t=2.63
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"Bash","input":{"command":"echo one","description":"Print one"}}],"…":"…"},"…":"…"}
# t=6.09
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"one","is_error":false}]},"…":"…"}
# t=6.58
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-2","name":"Bash","input":{"command":"echo two","description":"Print two"}}],"…":"…"},"…":"…"}
# t=6.58
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":2,"stop_reason":"tool_use","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000382)"],"queued_turn_count":0,"result_index":0,"…":"…"}
# t=8.99
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: alive"},"parent_tool_use_id":null},"_t":8.992}
# t=8.99
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=8.99
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":null,"terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000382)"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=9.20
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":9.201}
# t=10.30
{"_meta":{"exit_code":1},"_t":10.297}
```

Its two `result` lines (the turn that reached the limit, and the turn after it):

```json
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":2,"stop_reason":"tool_use","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000382)"],"total_cost_usd":0.0030055599999999996,"result_index":0,"model_usage_since_last_result":{"claude-haiku-5-5":{"costUSD":0.0004525}},"…":"…"}
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":null,"terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000382)"],"total_cost_usd":0.0030055599999999996,"result_index":1,"model_usage_since_last_result":{},"…":"…"}
```

`r6-budget-midturn-1-c1`: the model's answer and the `result` (the first try of the first recording):

```json
# t=5.18
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"I haven't run the commands. The budget notice says $0.000225 remains, and each tool call re-sends the conversation context, so three more calls could go over it. If you raise the budget or tell me to go ahead anyway, I'll run `echo one`, `echo two`, and `echo …"}],"…":"…"},"…":"…"}
# t=5.18
{"type":"result","subtype":"error_max_budget_usd","is_error":true,"num_turns":1,"stop_reason":"end_turn","terminal_reason":"budget_exhausted","errors":["Reached maximum budget ($0.000225)"],"queued_turn_count":0,"result_index":0,"…":"…"}
```

What the adapter can rely on (n=4 processes that reached the limit: 2 after one tool call had run, 2 before any):
- On a resumed process the limit works in the middle of a turn as on a new one (part 1): the turn ends with
  `{"type":"result","subtype":"error_max_budget_usd","is_error":true,"terminal_reason":"budget_exhausted",
  "errors":["Reached maximum budget ($…)"]}` (`stop_reason` is `tool_use` in three runs and `end_turn` in one).
- WHEN: right after a model request. In three of the four runs the last line before the `result` is an `assistant`
  line with a `tool_use`, and that tool call did NOT run (no tool result follows): in two of them one earlier tool
  call of the turn had run and returned, in one it was the turn's first. So the limit is looked at after each model
  request, and a tool call that the request asked for is dropped, not half done.
- In the fourth run the model asked for no tool: it answered in text that the budget notice leaves too little for
  three calls (the model is told the budget: part 3, section 14), and the limit came after that request
  (`stop_reason:"end_turn"`, the text is the `result`). So a small budget can also end a turn with nothing done and
  an answer that talks about the budget.
- The limit counts this process alone (part 3, section 14), and it is passed, not met: the turn's own cost was
  0.000452 USD against a budget of 0.000382 USD when it stopped (the request that crosses the limit is paid for).
- `total_cost_usd` of that `result` is the SESSION's total (0.003006 USD: the earlier processes of the session are in
  it), far above the budget. `model_usage_since_last_result` is the turn alone.
- The process stays alive after the limit. A second message gets `system/init`, then at once the same error `result`
  (`num_turns` 1, `stop_reason` null, the same `total_cost_usd`, no `model_usage_since_last_result`): no model request
  is made (no `assistant` line). After stdin close the exit code is 1.
- So a process that reached its budget is of no more use: every later turn fails at once and costs nothing. To go
  on, the host needs a new process with a new budget.
- How coarse the limit is: the two runs that stopped after their FIRST model request had spent 0.000284 and 0.000356 USD on it,
  while the whole turn of run b (four requests, no budget) cost 0.000449 USD. The first request of a resumed process is
  the dear one, so a budget below its cost stops the turn before any tool call.

## 11. The same server name in `--mcp-config` and in the user's own setup

Question: which one is loaded, and what do `init` and `mcp_status` say about `source`?

Fixtures: `r6-mcp-user-clash-1`, `-2` (`--permission-mode manual --setting-sources user --tools Bash --mcp-config /tmp/work/r6-mcp-user-clash-1.json`) and `r6-mcp-user-strict-1` (control: the same with `--strict-mcp-config`). A THROW-AWAY HOME whose `.claude.json` defines `ping-same` (it would answer `pong from user` and write `started-user.txt`); `--mcp-config` defines `ping-same` too (`pong from config`, `started-config.txt`). No login, so NO TOOL CALL is possible: the recordings show what is loaded.

First recording: `mcp_status` with its answer, the init line, the result, the driver's note:

```json
# t=0.00
{"_meta":{"env_set":{"HOME":"/tmp/work/r6-mcp-user-clash-1-home"}},"_t":0.0}
# t=3.02
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"mcp_status"}},"_t":3.025}
# t=3.02
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"mcpServers":[{"name":"ping-same","status":"connected","serverInfo":{"name":"ping-project","version":"0.0.1"},"config":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started-config.txt","label=config"]},"transport":"stdio","scope":"dynamic","source":"dynamic","tools":[{"name":"ping","annotations":{}}]}]}}}
# t=3.03
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","mcp__ping-same__ping"],"mcp_servers":[{"name":"ping-same","status":"connected","source":"dynamic"}],"…":"…"}
# t=3.03
{"type":"result","subtype":"success","is_error":true,"num_turns":1,"result":"Not logged in · Please run /login","stop_reason":"stop_sequence","terminal_reason":"api_error","queued_turn_count":0,"result_index":0,"…":"…"}
# t=3.24
{"_meta":{"server_starts":{"started-config.txt":1},"init_entries_for_this_name":[{"name":"ping-same","status":"connected","source":"dynamic"}],"other_servers_in_init":0,"mcp_status_entries_for_this_name":[{"name":"ping-same","status":"connected","scope":"dynamic","source":"dynamic"}],"result":"Not logged in · Please run /login"},"_t":3.239}
# t=3.24
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":3.241}
# t=3.80
{"_meta":{"exit_code":1},"_t":3.802}
```

What the adapter can rely on (n=2, and the control with `--strict-mcp-config` is the same):
- The `--mcp-config` definition wins: only `started-config.txt` exists (one start), the user's definition was not
  started at all.
- There is ONE entry of that name. `init.mcp_servers` says `{"name":"ping-same","status":"connected","source":"dynamic"}`
  and `mcp_status` says `"scope":"dynamic"`, `"source":"dynamic"`, with the config of the `--mcp-config` definition.
  Nothing in the stream says that a user definition of the same name exists or was shadowed.
- The same as the clash with a project `.mcp.json` (part 3, section 11): a server that the host passes with
  `--mcp-config` cannot be replaced by a definition of the same name from the user's setup or from the repo.
- Not recorded: the `can_use_tool` request for it (no login in a throw-away HOME). In every recorded request the
  `mcp_server.source` is the init entry's `source`, so `dynamic` is the expected word, not a recorded one.

## Can the waiting code be built now? (as of the day of recording)

| # | the code that waits | can it be built now? |
|---|---|---|
| 1 | The end of helpers with an interrupted turn (the Claude adapter's turn end; the store's `turn_completed`) | yes: nothing has to change for an interrupt. The stream ends each helper itself, background ones too (`task_updated` `killed`, `task_notification` `stopped` with the `Agent` call's `tool_use_id`, list `[]`), before the turn's `result` (6 of 6 recordings with a turn). The adapter already ends a helper at that line, so no background helper is kept past an interrupted turn, and the store's rule is confirmed for `interrupted`. A `task_notification` of a foreground Bash call (the Bash call's `tool_use_id`, empty `output_file`) belongs to no helper: the adapter already ends nothing for it. Open: a turn that FAILS by itself (the store's rule for `failed`) |
| 2 | `wind_down` (the stop of helpers at shutdown); a Stop button for foreground helpers | yes: `interrupt` first (also with no turn running) ends the helpers and starts no turn; then `stop_task` for commands; then close. `stop_task` on a foreground helper works and the turn goes on. Not to build: `stop_task` on a helper followed by stdin close (the self-started turn then runs to its end) |
| 3 | The end words of helper rows and background rows | yes: a helper that fails says `failed` (with `patch.error`); a command whose shell is killed says `failed` too. There is no word for a signal, and an empty list does not mean that no process is left |
| 4 | The restart that takes an effort level away | yes: `effortLevel:null` as the first request gives `medium` on a `--effort high` process (n=2). No restart is needed. Still not visible: the level a model request carries |
| 5 | `model_switched` (what to say about a line's model after a switch) | yes: a switch takes effect at the next model request. During an answer the rest of that answer stays on the old model; during a tool call the next assistant line is on the new one (part 3) |
| 6 | The approval card's sentence about a deny | yes: a deny is for that one command; the next command to the host asks again (4 of 4). Only an allow is remembered |
| 7 | Reconnect in the sheet; the state of a server that died | partly: Reconnect on a connected server is a restart and can be offered. A dead stdio server cannot be shown: `mcp_status` keeps saying `connected` and no line comes; the next call starts it again. A server that cannot start again: not recorded |
| 8 | Keeping your own MCP servers in an untrusted folder (`--strict-mcp-config` there) | partly. Recorded for both forms of the untrusted start: with `--setting-sources user` the repo's `.mcp.json` is not loaded and your own servers are (real login, n=1; no login, n=1); with `user,local` it is not loaded either, with no local settings file (no login, n=1) and with a local file that turns the project's servers on (real login, n=1; no login, n=2). One logged-in recording per form is thin for a safety switch: record each again before `--strict-mcp-config` goes. Not recorded: an approval kept in `~/.claude.json`; a local-scope server. `disabledMcpjsonServers` is the key that works when `project` is a source |
| 9 | Which resume failures give the conversation up (`SessionGone` against "any other failure") | partly: the two cases are not failures (9 of 9 starts succeed), so nothing new to tell apart. New risks to know: a resume from another folder finds the session, and a damaged file loses its tail without a word |
| 10 | Passing the rest of Codly's cost budget to the CLI | yes: the limit works mid-turn on a resumed process and counts the process alone. To build in: the limit is passed by up to one model request, the pending tool call is dropped, `total_cost_usd` is the session's, and the process is dead for further turns |
| 11 | The comment about a server of your own setup with the name of one of Codly's | partly: Codly's definition is the one that loads and the entry says `dynamic` (`init`, `mcp_status`). The request for its tool is not recorded (no login) |

## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | An interrupt while a helper works | (a) foreground: the answer, the `Agent` call rejected, `task_updated` `killed`, `task_notification` `stopped`, the turn's error `result`; no list line. (b) background: it does NOT go on: the same two task lines and a list line `[]` come before the answer, with or without a turn running; no turn starts by itself. A foreground Bash call that had become a task gets a `task_notification` of its own (`stopped`, empty `output_file`, no `task_updated`) | 2 + 4 + 4 (with section 2) | `r6-interrupt-helper-foreground-1`, `-2`, `-background-1`, `-2`, `-idle-1`, `-2` |
| 2 | A helper's stop inside a shutdown; `stop_task` on a foreground helper | `interrupt` ends the helper; `stop_task` after it is a no-op; exit 0.53 to 1.10 s after stdin close, nothing left. `stop_task` without an interrupt starts a turn that runs to its end even after stdin close (exit 1.60 / 1.73 s later). Foreground helper: `killed` / `stopped`, the `Agent` call returns an error result, the turn goes on | 2 per variant | `r6-shutdown-helper-sequence-1`, `-2`, `-idle-interrupt-*`, `-idle-close-now-*`, `-idle-interrupt-again-*`, `-idle-outwait-*`, `-foreground-stop-*` |
| 3 | A helper that fails; a command killed by a signal | Helper: `failed` (`patch.status` `failed`, `patch.error`), list `[]`, a self-started turn (background) or an error tool result (foreground). Command: SIGTERM or SIGKILL to its shell -> `failed` (`exit code 144` / `exit code 1`), list `[]`, the `sleep` child still alive; SIGKILL to the child -> `completed` | 1 per row | `r6-fails-helper-background`, `-helper-foreground`, `-signal-term`, `-signal-kill`, `-signal-child-kill` |
| 4 | `effortLevel:null` first, on `--effort high` | `applied.effort` `high` -> `medium` at once and through the next turn; `high` again only when sent | 2 | `r6-effort-null-first-fresh`, `-after-turn` |
| 5 | `set_model` while an answer streams | The rest of the answer and its `assistant` line stay on the old model, `modelUsage` has the old model only; the next turn is on the new one | 2 | `r6-model-while-streaming-1`, `-2` |
| 6 | A host denied once | The next command to the host is asked about again (4 of 4); after an allow it is not; no rule is written | 2 | `r6-sandbox-network-deny-1`, `-2` |
| 7 | `mcp_reconnect` on a connected server; a killed stdio server | Reconnect: success with no body, the server is started again. Killed: no line, `mcp_status` still `connected` 8 s later, the next call returns `pong` (the CLI started the server again by itself) | 2 | `r6-mcp-live-server-status-first`, `-call-first` |
| 8 | Project server, real login | `enableAllProjectMcpServers:false`, foreign `enabledMcpjsonServers`: loaded, tool called. `disabledMcpjsonServers`: kept out. `--setting-sources user` without `--strict-mcp-config`: not loaded, the user's own servers are. `user,local`: not loaded either, also when the folder's `.claude/settings.local.json` turns the project's servers on | 1 per row (the local file without a login: 2) | `r6-project-mcp-enable-all-false`, `-enabled-other`, `-disabled`, `-user-only`, `-user-only-home`, `-user-local-home`, `-user-local-file`, `-user-local-file-home-1`, `-2` (checks: `-hooks-on`, `-hooks-off`) |
| 9 | `--resume` from another folder; a damaged transcript | Both start: `initialize` answered, exit code 0 (9 of 9). Another folder: the session is found, the file stays in the first folder. Damaged: read up to the damage, no error | 3 per row | `r6-resume-source`, `r6-resume-other-folder-1` … `-3`, `r6-resume-damaged-tail-1` … `-3`, `r6-resume-damaged-mid-1` … `-3` |
| 10 | Budget reached mid-turn on a resumed process | `error_max_budget_usd` right after a model request: in 3 of 4 after a `tool_use` line whose call does not run, in 1 after a text answer; `total_cost_usd` is the session's; the next turn fails at once with the same result and no model request; the process stays alive | 4 (2 after a tool call) | `r6-budget-midturn-1-a` … `-1-c3`, `r6-budget-midturn-2-a` … `-2-c1` |
| 11 | Same name in `--mcp-config` and the user's setup | The `--mcp-config` one is loaded (one start), the user's is not started; `source` `dynamic` in `init` and `mcp_status`. No tool call (no login) | 2 (+1 control) | `r6-mcp-user-clash-1`, `-2`, `r6-mcp-user-strict-1` |

## Corrections to part 3

- Section 4 (the stop sequence) gave "0.21 s" from the interrupt to the stopped turn's `result` and "0.217 s" to the
  `stop_task` answer. Those were the driver's poll interval, not times of the CLI. The text of part 3 is corrected
  (through its builder). Measured here with the time stamp of each line: 0.032, 0.023 s from the interrupt to the `result`.
- Section 1 said that a host that stops a helper at shutdown "must stop that turn too or close before it". Closing
  stdin right after the answer to `stop_task` does not help: the self-started turn had begun by then and ran to its
  end after stdin was closed (section 2 here; a close BEFORE that turn's `system/init` is not in a kept recording).
  Send `interrupt` in place of `stop_task` for helpers.
- Section 4 said that a shutdown that stops a helper "has to interrupt or outwait that turn". With an `interrupt`
  as the first step there is no such turn (sections 1 and 2 here).

## Not verified

Not in this part (still open from the earlier lists):
- The text printed when `failIfUnavailable` stops a start (needs a machine where the sandbox cannot start).
- `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url` (a real sign-in; may write to the login keychain).
- `claude --resume` typed into the app's terminal on a session Codly started headless.
- Linux behaviour of process groups.
- A `tool_progress` line for WebFetch (needs a slow https page with a trusted certificate).
- A network allow that lasts hours (part 3 recorded 304 s).
- Whether a model request carries the effort level that `get_settings` shows.

Open after these recordings:
- 1: a turn that FAILS by itself (an API error, a limit) while a background helper works: does the helper go on?
  An interrupt while a helper has a background command of its own; a nested helper; `interrupt` with
  `cancel_queued` or `send_now` while a helper works. How long a Bash call must run before it becomes a task (not
  measured), and the `task_notification` of such a call when it ends by itself.
- 2: the same shutdown with a helper AND a background command at once; a helper that ignores the stop (none did);
  the exit when the self-started turn is long (here it was one short answer).
- 3: a helper that fails in another way (a tool error is not a failure of the helper; a refused request is the only
  one recorded); a command killed by SIGINT or SIGHUP; a command whose whole process group is killed.
- 4: `null` on a process started with `--effort low` or `max`; whether the next model request uses `medium`.
- 5: `set_model` during a thinking block, and between two requests of one turn with no tool running.
- 6: a deny with `"interrupt":true`; a deny that saves a rule; a deny in one process and the same host after a
  `--resume`.
- 7: a server that cannot be started again when its tool is called; a server that exits by itself with an error
  (here it was killed); an http server that goes away; how long `mcp_status` keeps saying `connected`.
- 8: a local-scope MCP server (`~/.claude.json` under the project) with `--setting-sources user`; a server from a
  plugin; an approval of the project's servers that is kept in `~/.claude.json` under the project's path;
  `user,local` with a local settings file that has only `enableAllProjectMcpServers` or only
  `enabledMcpjsonServers`; the logged-in `user` and `user,local` runs a second time (n=1 each).
- 9: a resume from a folder that is not next to the first one (another disk, another user's path); a transcript cut
  in the middle of an assistant line that holds a tool call; an EMPTY transcript file; a file with the right name
  and no valid line.
- 10: what the CLI tells the model about the budget; a limit that is reached while a helper works; a budget and a
  long tool call (is the call that runs cut off? here the limit came between calls).
- 11: the `can_use_tool` request for the server (needs a login with a user-scope server of that name); a clash
  between a plugin's server and `--mcp-config`.
