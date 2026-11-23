# Claude Code CLI stream-json: recorded protocol shapes, part 2

- CLI: `claude` 2.1.311 (the same version as part 1, `cli-claude-stream-json-2.1.311.md`; checked with `claude --version` before the runs).
- Recorded: 2026-11-22, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing").
  Fixtures of this part are `fixtures/claude-2.1.311/r4-*.ndjson`. The driver scripts of both parts are in
  `drivers/claude-stream-json/` (README there says how to record again).
- List price of the 35 recordings (sum of `total_cost_usd` of their `result` lines): 0.18 USD.
- Everything below was observed in these recordings. Where a statement comes from reading the CLI bundle and not from a
  recording, it says "bundle, not recorded". Counts are small: most scenarios are one run (n=1), so timings are examples,
  not distributions.

Differences from the part 1 method:
- The scenario flags are given per section (the part of `_argv` after the base flags). `--permission-mode` is `acceptEdits`
  unless the section says `manual` (what the app passes for a 1:1 agent; the CLI reports it as `default`).
- New sanitizing rules: a base64 image longer than 20,000 characters is replaced by `<base64 removed: N characters>`;
  in `system/init` and `mcp_status` only the MCP servers the scenario itself configured are kept (`needs-login`,
  `ping-project`), and `init.tools` drops the `mcp__…` tools of every other server. A `cf-ray` header value and the
  local time of a file listing are replaced. `demo@example.invalid` (the commit identity of the throw-away repos) is
  kept; every other email address becomes `user-email`.
- Process ids in `ps` output and in driver notes are real (they are needed to read the tables).
- In quoted lines `"…":"…"` stands for keys left out (assistant, user, result and init lines are shortened; the fixture
  has the full line). Lines starting with `# t=` are my time stamps (seconds since the process started): the last
  time marker of the fixture before that line, so the line was written then or up to 0.25 s later. (The part 2
  fixtures get a marker whenever 0.25 s passed since the last one; part 1's markers could lag behind during a long
  run of stream events.)

Two things seen in many of these recordings, said once here:
- The first message of a turn gets `command_lifecycle` lines too when it carries a `uuid`: `queued` and `started`
  when it is read, `completed` (or `cancelled`, when the turn is interrupted) right after the turn's `result`.
- `system/status` with `"status":"requesting"` carries `user_message_uuids`: the messages that this model request answers.

## A. Queue and stop

### 1. A message waits in the queue, then the host sends `interrupt`

Question: what does the interrupt response hold, which `command_lifecycle` lines come, and does the waiting message run or is it dropped?

Fixtures: `r4-queue-interrupt-plain`, `r4-queue-interrupt-cancel-queued`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

Sent: a turn with one `sleep 15`; 1 s after the tool started, a second user message; 1 s later the interrupt.

```json
{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `sleep 15` and then reply with the single word: done"},"parent_tool_use_id":null,"uuid":"user-msg-1"}
{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: banana"},"parent_tool_use_id":null,"uuid":"user-msg-2"}
{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}}
```

Plain `interrupt` (no fields): every line from the second message on, without the tool result lines:

```json
# t=3.07
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: banana"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":3.073}
# t=3.07
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=4.08
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":4.082}
# t=4.08
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":["user-msg-2"]}}}
# t=4.08
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1"],"…":"…"}
# t=4.08
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"cancelled"}
# t=4.08
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=4.08
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=4.56
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"banana","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"user_message_uuid":"user-msg-2","user_message_uuids":["user-msg-2"],"…":"…"}
# t=4.56
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
# t=8.69
{"_meta":{"results_seen":2},"_t":8.694}
```

`interrupt` with `"cancel_queued":true`:

```json
# t=3.07
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: banana"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":3.07}
# t=3.07
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=4.08
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt","cancel_queued":true}},"_t":4.081}
# t=4.08
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"cancelled"}
# t=4.08
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[],"cancelled":["user-msg-2"]}}}
# t=4.08
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1"],"…":"…"}
# t=4.08
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"cancelled"}
# t=38.30
{"_meta":{"results_seen":1},"_t":38.298}
```

What the adapter can rely on:
- `still_queued` is an array of plain uuid strings: the uuids the host put on the messages that still wait.
- A plain interrupt does NOT drop the waiting message. The stopped turn ends with its error `result`
  (`error_during_execution`, `terminal_reason:"aborted_tools"`), its own message gets `command_lifecycle` `cancelled`,
  and in the same instant (0.00 s later) the waiting message gets `started` and runs as a turn of its own (new
  `system/init`, second `result`, `result_index` 1; here 0.5 s after the interrupt).
- With `cancel_queued:true` the CLI drops the waiting message itself: `command_lifecycle` `cancelled` for it BEFORE
  the response, the response is `{"still_queued":[],"cancelled":[<uuid>]}`, only one `result` follows (the stopped
  turn's), and nothing ran in the 34 s after. The process stays usable (the next message ran).
  `init.capabilities` names this feature: `interrupt_cancel_queued_v1` (next to `interrupt_receipt_v1`,
  `msg_lifecycle_v1`, `interrupt_send_now_v1`, `interrupt_requeue_taken_v1`).
- The stopped turn's `result` has `queued_turn_count` 0 in both runs, with one message waiting: do not read that
  field as "how many messages wait".

### 2. Two user messages written mid-turn

Question: order of `command_lifecycle`, one result or several, `result.user_message_uuids`.

Fixtures: `r4-two-queued-tools` (the turn has tool boundaries: three `sleep 4`), `r4-two-queued-notool` (a text-only turn). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

With tool boundaries (lifecycle lines, tool results, final text, result):

```json
# t=0.00
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `sleep 4` three times, one tool call after the other (never in parallel, never combined in one command). After the third one finishes, reply with the single word: done"},"parent_tool_use_id":null,"uuid":"user-msg-1"},"_t":0.004}
# t=1.15
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"queued"}
# t=1.15
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"started"}
# t=1.15
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=3.04
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"also say the word banana at the end"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":3.041}
# t=3.04
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=3.35
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"and the word cherry after that"},"parent_tool_use_id":null,"uuid":"user-msg-3"},"_t":3.348}
# t=3.35
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"queued"}
# t=8.72
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=8.72
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=8.72
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"started"}
# t=14.42
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=19.12
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-3","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=19.61
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"done banana cherry"}],"…":"…"},"…":"…"}
# t=19.61
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
# t=19.61
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"completed"}
# t=19.61
{"type":"result","subtype":"success","is_error":false,"num_turns":4,"result":"done banana cherry","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1","user-msg-2","user-msg-3"],"…":"…"}
# t=19.61
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"completed"}
```

Text-only turn (no boundary to fold at): both messages wait for the turn's end:

```json
# t=0.01
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Write the numbers from 1 to 150, one per line, and nothing else."},"parent_tool_use_id":null,"uuid":"user-msg-1"},"_t":0.008}
# t=0.89
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"queued"}
# t=0.89
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"started"}
# t=0.89
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=0.89
{"type":"system","subtype":"status","status":"requesting","user_message_uuids":["user-msg-1"]}
# t=1.59
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"also say the word banana at the end"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":1.589}
# t=1.59
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=1.90
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"and the word cherry after that"},"parent_tool_use_id":null,"uuid":"user-msg-3"},"_t":1.895}
# t=1.90
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"queued"}
# t=2.15
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n1…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1"],"…":"…"}
# t=2.15
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"completed"}
# t=2.15
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=2.15
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"started"}
# t=2.41
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=2.41
{"type":"system","subtype":"status","status":"requesting","user_message_uuids":["user-msg-2","user-msg-3"]}
# t=3.68
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n16\n1…","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"user_message_uuid":"user-msg-3","user_message_uuids":["user-msg-2","user-msg-3"],"…":"…"}
# t=3.68
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
# t=3.68
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"completed"}
```

What the adapter can rely on:
- Both messages get `queued` at once, in the order written. At the next tool boundary both get `started`, in the same
  order, and the turn goes on with both: ONE `result`, `user_message_uuid` = the message that opened the turn,
  `user_message_uuids` = all three in order.
- `completed` for the folded messages comes just BEFORE the `result`; `completed` for the turn's own message comes
  just after it.
- With no tool boundary before the turn's end, the two waiting messages become ONE new turn together: after the first
  `result`, `started` for both, one `system/init`, one model request (`status` `requesting` lists both uuids), one
  `result` whose `user_message_uuids` is both and whose `user_message_uuid` is the LAST of them.
  So: several queued messages never make several turns by themselves; the number of `result` lines is the number of
  turns, not of messages.

### 3. A slash command written mid-turn

Question: is it folded into the turn, run after it, or refused?

Fixtures: `r4-slash-mid-turn-context` (`/context`), `r4-slash-mid-turn-compact` (`/compact`). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`. The turn runs `sleep 4` twice; the command is written during the first.

```json
# t=0.00
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `sleep 4` two times, one tool call after the other (never in parallel, never combined in one command). After the second one finishes, reply with the single word: done"},"parent_tool_use_id":null,"uuid":"user-msg-1"},"_t":0.004}
# t=1.01
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"queued"}
# t=1.01
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"started"}
# t=1.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=2.90
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"/context"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":2.895}
# t=2.90
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=8.32
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=13.07
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=13.56
{"type":"result","subtype":"success","is_error":false,"num_turns":3,"result":"done","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1"],"…":"…"}
# t=13.56
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"completed"}
# t=13.56
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=13.81
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=13.81
{"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"## Context Usage\n\n**Model:** claude-haiku-5-5  \n**Tokens:** …","stop_reason":null,"local_command":"context","queued_turn_count":0,"result_index":1,"user_message_uuid":"user-msg-2","user_message_uuids":["user-msg-2"],"…":"…"}
# t=13.81
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
```

`/compact` (from the first turn's result on):

```json
# t=13.28
{"type":"result","subtype":"success","is_error":false,"num_turns":3,"result":"done","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1"],"…":"…"}
# t=13.28
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"completed"}
# t=13.28
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=13.28
{"type":"system","subtype":"status","status":"compacting"}
# t=17.90
{"type":"system","subtype":"status","status":null,"compact_result":"success"}
# t=17.90
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=17.90
{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":5937,"post_tokens":1022,"cumulative_dropped_tokens":4915,"duration_ms":4559,"preserved_segment":{"head_uuid":"uuid-44","anchor_uuid":"uuid-55","tail_uuid":"uuid-44"},"preserved_messages":{"anchor_uuid":"uuid-55","uuids":["uuid-44"],"all_uuids":["uuid-44"]}},"logical_parent_uuid":"uuid-44"}
# t=17.90
{"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","stop_reason":null,"local_command":"compact","queued_turn_count":0,"result_index":1,"user_message_uuid":"user-msg-2","user_message_uuids":["user-msg-2"],"…":"…"}
# t=17.90
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
```

What the adapter can rely on:
- A slash command written mid-turn is accepted (`queued`), is NOT folded at a tool boundary (two boundaries passed),
  and runs as a turn of its own right after the running turn's `result`: `started`, the command's normal lines
  (part 1, section 11), its own `result` with `local_command`, `completed`.
- So the CLI already does what the adapter does by holding a slash command back until the turn ends.

### 4. A background Bash task ends while a user message waits

Question: which turn takes the message?

Fixtures: `r4-background-and-queue-fold` (message with no priority), `r4-background-and-queue-later` (`priority:"later"`). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

Turn 1 starts `sleep 12; echo woke` in the background. Turn 2 runs a foreground `sleep 14`; 1 s into it the third message is written; the background task ends 9.6 s later, while the message waits.

No priority (from the start of turn 2):

```json
# t=4.34
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `sleep 14` (in the foreground) and then reply with the single word: two"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":4.34}
# t=4.34
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=4.34
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=4.34
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=6.08
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: three"},"parent_tool_use_id":null,"uuid":"user-msg-3"},"_t":6.08}
# t=6.08
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"queued"}
# t=8.09
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"Sleep for 14 seconds in the foreground","is_backgrounded":false,"task_type":"local_bash"}
# t=15.70
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795395854733}}
# t=15.70
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r4-background-and-queue-fold/sess-1/tasks/tid-1.output","summary":"Background command \"sleep 12; echo woke\" completed (exit code 0)"}
# t=15.70
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=19.11
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"completed","output_file":"","summary":"Sleep for 14 seconds in the foreground"}
# t=19.11
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=19.11
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"started"}
# t=20.53
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"two\n\nThe tool result also contained text asking me to reply \"three\". That isn't part of your request, so I ignored it. The background `sleep 12` task from earlier has also finished."}],"…":"…"},"…":"…"}
# t=20.53
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"completed"}
# t=20.53
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"two\n\nThe tool result also contained text asking me to reply \"three\". That isn't part of your request, so I ignored it. The background `sleep 12` task from earlier has also finished.","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"user_message_uuid":"user-msg-2","user_message_uuids":["user-msg-2","user-msg-3"],"…":"…"}
# t=20.53
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
```

`priority:"later"` (from the start of turn 2):

```json
# t=4.33
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run the Bash command `sleep 14` (in the foreground) and then reply with the single word: two"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":4.325}
# t=4.33
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=4.33
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"started"}
# t=4.33
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=6.08
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Reply with the single word: three"},"parent_tool_use_id":null,"uuid":"user-msg-3","priority":"later"},"_t":6.076}
# t=6.08
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"queued"}
# t=8.08
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"Sleep for 14 seconds in the foreground","is_backgrounded":false,"task_type":"local_bash"}
# t=15.66
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795395854647}}
# t=15.66
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r4-background-and-queue-later/sess-1/tasks/tid-1.output","summary":"Background command \"sleep 12; echo woke\" completed (exit code 0)"}
# t=15.66
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=19.10
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"completed","output_file":"","summary":"Sleep for 14 seconds in the foreground"}
# t=19.10
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=19.84
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"two"}],"…":"…"},"…":"…"}
# t=19.84
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"two","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"user_message_uuid":"user-msg-2","user_message_uuids":["user-msg-2"],"…":"…"}
# t=19.84
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"completed"}
# t=19.84
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"started"}
# t=19.84
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=20.57
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"three"}],"…":"…"},"…":"…"}
# t=20.57
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"three","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":2,"user_message_uuid":"user-msg-3","user_message_uuids":["user-msg-3"],"…":"…"}
# t=20.57
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"completed"}
```

What the adapter can rely on:
- The background task's end is written at once (`task_updated`, `task_notification`, `background_tasks_changed` with
  an empty list), also in the middle of a turn. It does not start a turn while one runs.
- No priority: the waiting message is taken into the RUNNING turn at the next tool boundary (`started` right after the
  foreground tool's result). One `result` for that turn, `user_message_uuids` has both messages. The task's
  notification went into the same turn (the answer mentions the finished background task). No turn started by itself
  afterwards (watched for 68 s).
- `priority:"later"`: the running turn ends first (it also took the notification), then the message runs as its own
  turn. Again no self-started turn (watched for 28 s).
- In both runs there is no extra turn for the task notification: a notification that arrives while a turn runs is
  used up by that turn. The self-started turn of part 1 (section 5) only happens when the CLI is idle.
- One model behaviour to know (n=1): in the no-priority run the model answered only `two` and wrote that the tool
  result "also contained text asking me to reply three", which it ignored. A message folded in at a tool boundary can
  be read by the model as text that came with the tool result. In scenarios 2 and 5 the folded messages were followed.

### 5. `cancel_async_message` on a really queued message

Question: request and response shape, and what else is written.

Fixture: `r4-cancel-async-message`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`. The turn runs `sleep 6` twice; two messages are written during the first.

```json
# t=3.10
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"also say the word banana at the end"},"parent_tool_use_id":null,"uuid":"user-msg-2"},"_t":3.096}
# t=3.10
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"and the word cherry after that"},"parent_tool_use_id":null,"uuid":"user-msg-3"},"_t":3.096}
# t=3.10
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"queued"}
# t=3.10
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"queued"}
# t=3.10
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"cancel_async_message","message_uuid":"user-msg-2"}},"_t":3.098}
# t=3.10
{"type":"command_lifecycle","command_uuid":"user-msg-2","state":"cancelled"}
# t=3.10
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"cancelled":true}}}
# t=3.10
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"cancel_async_message","message_uuid":"user-msg-2"}},"_t":3.098}
# t=3.10
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"cancelled":false}}}
# t=10.57
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=10.57
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"started"}
# t=10.58
{"_sent":{"type":"control_request","request_id":"host-3","request":{"subtype":"cancel_async_message","message_uuid":"user-msg-3"}},"_t":10.582}
# t=10.58
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"cancelled":false}}}
# t=10.59
{"_sent":{"type":"control_request","request_id":"host-4","request":{"subtype":"cancel_async_message","message_uuid":"user-msg-1"}},"_t":10.588}
# t=10.59
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"cancelled":false}}}
# t=17.80
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"…":"…"}
# t=18.34
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"done cherry"}],"…":"…"},"…":"…"}
# t=18.34
{"type":"command_lifecycle","command_uuid":"user-msg-3","state":"completed"}
# t=18.34
{"type":"result","subtype":"success","is_error":false,"num_turns":3,"result":"done cherry","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"user_message_uuid":"user-msg-1","user_message_uuids":["user-msg-1","user-msg-3"],"…":"…"}
# t=18.34
{"type":"command_lifecycle","command_uuid":"user-msg-1","state":"completed"}
```

What the adapter can rely on:
- Request: `{"subtype":"cancel_async_message","message_uuid":<uuid>}`. Answer: `{"cancelled":true|false}`.
- On a message that still waits: `command_lifecycle` `cancelled` for it is written BEFORE the response
  `{"cancelled":true}`. The message never reaches the model (the answer was `done cherry`, no `banana`) and is not in
  `result.user_message_uuids`.
- `{"cancelled":false}`, with no other line: for the same uuid again, for a message a turn already took (`started`
  was written), and for the message that opened the running turn.
- A host that withdraws a message this way must forget the uuid before it sends the request: the `cancelled` line
  comes first, and it looks exactly like the `cancelled` of a message the CLI gave up by itself.

## B. Background work and the end of a turn

### 6. The full life of a background task, `get_task_output`, `stop_task`

Question: which lines tell the host that a background task is open, when it started and when it ended? Is there a control request that lists open tasks? What does `stop_task` do on a real task id?

Fixture: `r4-background-life`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

Task 1 (`sleep 8; echo woke`, `run_in_background:true`) ends by itself. Every task line, control request and result up to the start of task 2:

```json
# t=0.59
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=2.40
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-1","run_id":"run-1","task_type":"local_bash","description":"sleep 8; echo woke"}]}
# t=2.40
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"sleep 8; echo woke","is_backgrounded":true,"task_type":"local_bash"}
# t=2.40
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"Command running in background with ID: tid-1. Output is being written to: /tmp/claude-tmp/-tmp-work-r4-background-life/sess-1/tasks/tid-1.output. You will be notified when it completes. To check interim output, use Read on that file path.","is_error":false}]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false,"backgroundTaskId":"tid-1"},"tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=2.88
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"started","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=4.02
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"get_task_output","task_id":"tid-1"}},"_t":4.024}
# t=4.02
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"output":"","total_bytes":0,"truncated":false}}}
# t=4.03
{"_sent":{"type":"control_request","request_id":"host-2","request":{"subtype":"background_tasks"}},"_t":4.027}
# t=4.03
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{}}}
# t=10.43
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795396030220}}
# t=10.43
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r4-background-life/sess-1/tasks/tid-1.output","summary":"Background command \"sleep 8; echo woke\" completed (exit code 0)"}
# t=10.43
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=10.43
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=11.07
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The background command `sleep 8; echo woke` finished with exit code 0.","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":1,"…":"…"}
# t=12.21
{"_sent":{"type":"control_request","request_id":"host-3","request":{"subtype":"get_task_output","task_id":"tid-1"}},"_t":12.209}
# t=12.21
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"output":"woke\n\n[exited with code 0]\n","total_bytes":27,"truncated":false}}}
# t=12.21
{"_sent":{"type":"control_request","request_id":"host-4","request":{"subtype":"stop_task","task_id":"tid-1"}},"_t":12.21}
# t=12.21
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{}}}
```

Task 2 (`sleep 127; echo late`) is stopped by the host 2 s after the turn ended:

```json
# t=12.21
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=12.68
{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tid-2","run_id":"run-2","task_type":"local_bash","description":"sleep 127; echo late"}]}
# t=12.68
{"type":"system","subtype":"task_started","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","description":"sleep 127; echo late","is_backgrounded":true,"task_type":"local_bash"}
# t=12.68
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"Command running in background with ID: tid-2. Output is being written to: /tmp/claude-tmp/-tmp-work-r4-background-life/sess-1/tasks/tid-2.output. You will be notified when it completes. To check interim output, use Read on that file path.","is_error":false}]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false,"backgroundTaskId":"tid-2"},"tool_result_meta":[{"id":"toolu-2","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=13.28
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"started","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":2,"…":"…"}
# t=15.53
{"_meta":{"host_sees_sleep_127_before_stop":true},"_t":15.525}
# t=15.53
{"_sent":{"type":"control_request","request_id":"host-5","request":{"subtype":"stop_task","task_id":"tid-2"}},"_t":15.525}
# t=15.53
{"type":"system","subtype":"task_updated","task_id":"tid-2","run_id":"run-2","patch":{"status":"killed","end_time":1795396035321}}
# t=15.53
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"/tmp/claude-tmp/-tmp-work-r4-background-life/sess-1/tasks/tid-2.output","summary":"sleep 127; echo late"}
# t=15.53
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=15.53
{"type":"control_response","response":{"subtype":"success","request_id":"host-5","response":{}}}
# t=30.74
{"_meta":{"host_sees_sleep_127_after_stop":false,"result_after_stop_task":false},"_t":30.741}
# t=30.74
{"_sent":{"type":"control_request","request_id":"host-6","request":{"subtype":"get_task_output","task_id":"tid-2"}},"_t":30.741}
# t=30.74
{"type":"control_response","response":{"subtype":"success","request_id":"host-6","response":{"output":"\n[killed]\n","total_bytes":10,"truncated":false}}}
# t=30.74
{"_sent":{"type":"control_request","request_id":"host-7","request":{"subtype":"stop_task","task_id":"tid-2"}},"_t":30.743}
# t=30.74
{"type":"control_response","response":{"subtype":"success","request_id":"host-7","response":{}}}
# t=32.74
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":32.745}
# t=33.47
{"_meta":{"exit_code":0},"_t":33.466}
```

What the adapter can rely on:
- Open: `system/task_started` with `"is_backgrounded":true` (`task_id`, `run_id`, `tool_use_id`, `description`,
  `task_type:"local_bash"`) and `system/background_tasks_changed`, whose `tasks` array is the full list of open
  background tasks at that moment (here it came just before `task_started`; in part 1 just after). The turn's
  `result` says nothing about open tasks.
- Ended by itself: `task_updated` (`patch.status:"completed"`, `end_time`), `task_notification`
  (`status:"completed"`, `output_file`, `summary`), `background_tasks_changed` with `tasks:[]`, then the self-started
  turn (`system/init` … `result` with `origin.kind:"task-notification"`).
- No control request lists open tasks. `background_tasks` answers `{}`: per the bundle it moves running FOREGROUND
  tasks to the background (bundle, not recorded with a foreground task). The list to keep is the last
  `background_tasks_changed.tasks`.
- `get_task_output` `{"task_id"}` answers `{"output","total_bytes","truncated"}`: the tail of what the command
  wrote. Running and silent: `""`. Ended: `"woke\n\n[exited with code 0]\n"`. Stopped: `"\n[killed]\n"`.
  It works for a foreground Bash call too, once its `task_started` line gave the id (scenario 8).
- `stop_task` `{"task_id"}` on a running task: `task_updated` (`patch.status:"killed"`), `task_notification` with
  `status:"stopped"`, `background_tasks_changed` `[]`, THEN the response `{}`. The command's process was gone when
  the host looked 15 s later. NO turn starts by itself after a stop (none in those 15 s).
- `stop_task` answers `{}` (success) also for a task that already ended, for one stopped before, and for an unknown
  id (part 1): the answer does not say whether anything was stopped; the task lines do.

### 7. Process groups of Bash children, and what outlives the CLI

Question: which process group do Bash children get, and are they alive after the CLI exits (stdin closed) and after SIGTERM to the CLI?

Fixtures: `r4-process-groups-blocked`, `r4-process-groups-stdin`, `r4-process-groups-sigterm`, `r4-process-groups-sigkill`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`. The driver starts the CLI as the leader of its own process group (`process_group=0`, as the app does).

First finding: the two commands as first planned never ran. The CLI refuses a Bash command whose first part is a long `sleep` (`r4-process-groups-blocked`):

```json
# t=0.01
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command (foreground, as written, one attempt only): sleep 301 & echo $! > bg.pid"},"parent_tool_use_id":null},"_t":0.005}
# t=1.76
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"Bash","input":{"command":"sleep 301 & echo $! > bg.pid","description":"Start sleep 301 in background and write its PID to bg.pid","timeout":120000}}],"…":"…"},"…":"…"}
# t=1.76
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"<tool_use_error>Blocked: sleep 301 followed by: echo $!. To wait for a condition, use Monitor with an until-loop (e.g. `until <check>; do sleep 2; done`). To wait for a command you started, use run_in_background: true. Do not chain shorter sleeps to work around this block.</tool_use_error>","is_error":true,"tool_use_id":"toolu-1"}]},"tool_use_result":"Error: Blocked: sleep 301 followed by: echo $!. To wait for a condition, use Monitor with an until-loop (e.g. `until <check>; do sleep 2; done`). To wait for a command you started, use run_in_background: true. Do not chain shorter sleeps to work around this block.","…":"…"}
# t=4.85
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command (foreground, as written, one attempt only): sleep 301 &"},"parent_tool_use_id":null},"_t":4.851}
# t=6.16
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-2","name":"Bash","input":{"command":"sleep 301 &","description":"Run sleep 301 in the background as a single attempt","timeout":120000}}],"…":"…"},"…":"…"}
# t=6.16
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"<tool_use_error>Blocked: standalone sleep 301. To wait for a condition, use Monitor with an until-loop (e.g. `until <check>; do sleep 2; done`). To wait for a command you started, use run_in_background: true. Do not chain shorter sleeps to work around this block.</tool_use_error>","is_error":true,"tool_use_id":"toolu-2"}]},"tool_use_result":"Error: Blocked: standalone sleep 301. To wait for a condition, use Monitor with an until-loop (e.g. `until <check>; do sleep 2; done`). To wait for a command you started, use run_in_background: true. Do not chain shorter sleeps to work around this block.","…":"…"}
```

The bundle's rule (bundle, not recorded as a number): the first sub-command matches `^sleep\s+(\d+(?:\.\d*)?)\s*$` and the number is 25 or more. `sleep 20` and `sleep 21` ran (scenario 8). So the runs below use `echo start; sleep 301 &`, plus a `sleep 47` with `run_in_background:true`.

The `ps` the model ran in a foreground Bash call (`r4-process-groups-stdin`; `50599` is the `sleep 301`, found by the host):

```json
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-3","type":"tool_result","content":"  PID  PGID  PPID COMMAND\n50370 50370 50329 /opt/homebrew/bin/claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio --permission-mode acceptEdits --session-id sess-1 --model haiku --strict-mcp-config --setting-sources local --tools Bash\n50599 50597     1 sleep 301\n50780 50780 50370 /bin/zsh -c source /Users/me/.claude/shell-snapshots/snapshot-zsh-1795396175521-d0i5fu.sh 2>/dev/null || true && setopt NO_EXTENDED_GLOB NO_BARE_GLOB_QUAL 2>/dev/null || true && { \\builtin unalias -- 'unsetenv'; \\builtin unset -f -- 'unsetenv'; } >/dev/null 2>&1 || true && eval 'ps -o pid,pgid,ppid,command -p 50599,$$,$PPID' < /dev/null && pwd -P >| /tmp/claude-5869-cwd","is_error":false}]},"…":"…"}
```

The host's own `ps` of the same processes while the CLI is alive, then what is left after the CLI ended, per variant (driver notes):

`r4-process-groups-stdin`:

```json
{"_meta":{"cli_pid":50370,"cli_pgid":50370,"driver_pgid":49954},"_t":0.005}
{"_meta":{"when":"CLI alive","host_ps":[{"pid":50370,"pgid":50370,"ppid":50329,"sess":"0","stat":"S","command":"claude (the CLI)"},{"pid":50599,"pgid":50597,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"},{"pid":50709,"pgid":50709,"ppid":50370,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"},{"pid":50712,"pgid":50709,"ppid":50709,"sess":"0","stat":"S","command":"sleep 47"}],"children_of_cli":[{"pid":50709,"pgid":50709,"ppid":50370,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"}],"group_of_cli":[{"pid":50370,"pgid":50370,"ppid":50329,"sess":"0","stat":"S","command":"claude (the CLI)"}]},"_t":9.867}
{"_meta":{"closing_stdin":true},"_t":9.892}
{"_meta":{"exit_code":0,"seconds_from_stdin_close_to_exit":43.54,"when":"1 s after the CLI ended","host_ps":[{"pid":50599,"pgid":50597,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"}]},"_t":54.467}
```

`r4-process-groups-sigterm`:

```json
{"_meta":{"cli_pid":52592,"cli_pgid":52592,"driver_pgid":49954},"_t":0.005}
{"_meta":{"when":"CLI alive","host_ps":[{"pid":52592,"pgid":52592,"ppid":52510,"sess":"0","stat":"S","command":"claude (the CLI)"},{"pid":52832,"pgid":52830,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"},{"pid":52964,"pgid":52964,"ppid":52592,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"},{"pid":52965,"pgid":52964,"ppid":52964,"sess":"0","stat":"S","command":"sleep 47"}],"children_of_cli":[{"pid":52964,"pgid":52964,"ppid":52592,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"}],"group_of_cli":[{"pid":52592,"pgid":52592,"ppid":52510,"sess":"0","stat":"S","command":"claude (the CLI)"}]},"_t":9.941}
{"_meta":{"sending":"SIGTERM to the CLI pid only"},"_t":9.966}
{"_meta":{"exit_code":143,"seconds_from_sigterm_to_exit":0.75,"when":"1 s after the CLI ended","host_ps":[{"pid":52832,"pgid":52830,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"}]},"_t":11.747}
```

`r4-process-groups-sigkill`:

```json
{"_meta":{"cli_pid":53633,"cli_pgid":53633,"driver_pgid":49954},"_t":0.005}
{"_meta":{"when":"CLI alive","host_ps":[{"pid":53633,"pgid":53633,"ppid":53510,"sess":"0","stat":"S","command":"claude (the CLI)"},{"pid":53867,"pgid":53866,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"},{"pid":54000,"pgid":54000,"ppid":53633,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"},{"pid":54002,"pgid":54000,"ppid":54000,"sess":"0","stat":"S","command":"sleep 47"}],"children_of_cli":[{"pid":54000,"pgid":54000,"ppid":53633,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"}],"group_of_cli":[{"pid":53633,"pgid":53633,"ppid":53510,"sess":"0","stat":"S","command":"claude (the CLI)"}]},"_t":9.354}
{"_meta":{"sending":"SIGKILL to the CLI pid only"},"_t":9.379}
{"_meta":{"exit_code":-9,"seconds_from_sigkill_to_exit":0.0,"when":"1 s after the CLI ended","host_ps":[{"pid":53867,"pgid":53866,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"},{"pid":54000,"pgid":54000,"ppid":1,"sess":"0","stat":"Ss","command":"/bin/zsh -c source /Users/me/.claude/shell-snapshots/sn"},{"pid":54002,"pgid":54000,"ppid":54000,"sess":"0","stat":"S","command":"sleep 47"}]},"_t":10.416}
```

After stdin was closed, with the `sleep 47` task still open (`r4-process-groups-stdin`): the lines the CLI wrote before it exited:

```json
# t=9.89
{"_meta":{"closing_stdin":true},"_t":9.892}
# t=52.10
{"type":"system","subtype":"task_updated","task_id":"tid-1","run_id":"run-1","patch":{"status":"completed","end_time":1795396226310}}
# t=52.10
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-2","status":"completed","output_file":"/tmp/claude-tmp/-tmp-work-r4-process-groups-stdin/sess-1/tasks/tid-1.output","summary":"Background command \"Start a 47-second sleep in the background\" completed (exit code 0)"}
# t=52.10
{"type":"system","subtype":"background_tasks_changed","tasks":[]}
# t=52.10
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=53.00
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"The background `sleep 47` finished with exit code 0.","stop_reason":"end_turn","terminal_reason":"completed","origin":{"kind":"task-notification","producer":"session-task","runId":"run-1"},"queued_turn_count":0,"result_index":3,"…":"…"}
# t=54.47
{"_meta":{"exit_code":0,"seconds_from_stdin_close_to_exit":43.54,"when":"1 s after the CLI ended","host_ps":[{"pid":50599,"pgid":50597,"ppid":1,"sess":"0","stat":"SN","command":"sleep 301"}]},"_t":54.467}
```

What the adapter can rely on:
- Every Bash call runs in a shell that is a child of the CLI (`ppid` = CLI) and the leader of its OWN process group
  and session (`pgid` = the shell's pid; `stat` has `s`, the session-leader mark; the `sess` column of macOS `ps`
  is always 0 and says nothing): `/bin/zsh -c source <shell snapshot> … && eval '<command>'`.
  This holds for a foreground call and for a `run_in_background` call. The CLI's process group holds the CLI alone
  (`group_of_cli`). A signal to the CLI's group (`killpg`) therefore reaches no shell and no command.
- A child left behind with `&` (`sleep 301`) keeps the group id of its shell, which has exited; its parent is pid 1.
  The CLI does not track it: it was alive after stdin close + exit 0, after SIGTERM and after SIGKILL of the CLI.
  Only a kill by pid (or by that old group id) ends it.
- A `run_in_background` task: SIGTERM to the CLI ends the CLI in 0.75 s with exit code 143 and the task's shell and
  command are gone 1 s later (the CLI kills what it tracks). SIGKILL to the CLI leaves the task's shell and command
  alive under pid 1.
- Closing stdin while a background task is open does NOT end the CLI: it stayed alive until the task ended (43.5 s),
  wrote the task lines, ran the self-started turn (a `result` 43 s after stdin was closed, `result_index` 3), and
  only then exited with code 0. A host that closes stdin to end the CLI must be ready for that wait and for that
  turn, or send SIGTERM.
- The Bash tool call `echo start; sleep 301 &` returned at once with `start` (the `&` child does not hold the call).
- The shell sources a snapshot of the user's shell setup, so aliases apply inside Bash calls (seen as `[bat error]`
  from `cat` and an `eza`-style listing from `ls` in other fixtures).

### 8. Lines during a long tool call, and an interrupt mid-tool

Question: which lines arrive while a tool runs, how often? How long from the `interrupt` request to the `result`?

Fixtures: `r4-tool-progress` (`sleep 20`, then `sleep 21` interrupted after 8 s), `r4-tool-progress-output` (a 96 s loop that prints a line every 4 s). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`.

`sleep 20`: every line from the `tool_use` line to the tool result, stream events included (the three that close the model's message, then only the two task lines):

```json
# t=1.18
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"Bash","input":{"command":"sleep 20","description":"Sleep for 20 seconds"}}],"…":"…"},"…":"…"}
# t=1.18
{"type":"stream_event","event":{"type":"content_block_stop","index":0},"session_id":"sess-1","parent_tool_use_id":null,"uuid":"uuid-12","api_message_id":"msg-1"}
# t=1.18
{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null,"stop_details":null,"container":null},"usage":"…","context_management":{"applied_edits":[]}},"session_id":"sess-1","parent_tool_use_id":null,"uuid":"uuid-13","api_message_id":"msg-1"}
# t=1.18
{"type":"stream_event","event":{"type":"message_stop"},"session_id":"sess-1","parent_tool_use_id":null,"uuid":"uuid-14","api_message_id":"msg-1"}
# t=5.42
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Sleep for 20 seconds","is_backgrounded":false,"task_type":"local_bash"}
# t=22.44
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"","summary":"Sleep for 20 seconds"}
# t=22.44
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false},"tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
```

The 96 s command: task lines, `tool_progress` lines, the `get_task_output` request sent 6 s after `task_started`, the result:

```json
# t=5.66
{"type":"system","subtype":"task_started","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","description":"Print tick numbers 1-24 with 4-second pauses between them","is_backgrounded":false,"task_type":"local_bash"}
# t=11.67
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"get_task_output","task_id":"tid-1"}},"_t":11.667}
# t=11.67
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"output":"tick 1\ntick 2\ntick 3\n","total_bytes":21,"truncated":false}}}
# t=31.68
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-0","tool_name":"Bash","parent_tool_use_id":"toolu-1","elapsed_time_seconds":30,"heartbeat":true}
# t=61.68
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-1","tool_name":"Bash","parent_tool_use_id":"toolu-1","elapsed_time_seconds":60,"heartbeat":true}
# t=91.68
{"type":"tool_progress","tool_use_id":"toolu-1-heartbeat-2","tool_name":"Bash","parent_tool_use_id":"toolu-1","elapsed_time_seconds":90,"heartbeat":true}
# t=98.98
{"type":"system","subtype":"task_notification","task_id":"tid-1","run_id":"run-1","tool_use_id":"toolu-1","status":"completed","output_file":"","summary":"Print tick numbers 1-24 with 4-second pauses between them"}
# t=99.56
{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"done","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
```

Interrupt 8 s into `sleep 21` (from the request to the driver's notes):

```json
# t=31.80
{"_meta":{"sleep_21_alive_before_interrupt":true},"_t":31.799}
# t=31.80
{"_sent":{"type":"control_request","request_id":"host-1","request":{"subtype":"interrupt"}},"_t":31.799}
# t=31.80
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"still_queued":[]}}}
# t=31.80
{"type":"system","subtype":"task_notification","task_id":"tid-2","run_id":"run-2","tool_use_id":"toolu-2","status":"stopped","output_file":"","summary":"Sleep for 21 seconds"}
# t=31.80
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","is_error":true,"tool_use_id":"toolu-2"}]},"tool_use_result":"User rejected tool use","tool_result_meta":[{"id":"toolu-2","non_execution_kind":"user-rejected"}],"…":"…"}
# t=31.80
{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]},"…":"…"}
# t=31.80
{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":3,"stop_reason":"tool_use","terminal_reason":"aborted_tools","errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"],"queued_turn_count":0,"result_index":1,"…":"…"}
# t=33.06
{"_meta":{"interrupt_response_after_s":0.003,"result_after_s":0.206,"sleep_21_alive_1s_after_interrupt":false},"_t":33.06}
# t=33.06
{"_meta":{"closing_stdin":true,"alive_before_close":true},"_t":33.06}
# t=33.83
{"_meta":{"exit_code":1},"_t":33.826}
```

What the adapter can rely on:
- While a Bash call runs the CLI writes: `system/task_started` (`is_backgrounded:false`) 3 to 4 s after the
  `tool_use` line (a call that ends sooner gets none: over all part 2 recordings, none of the 39 foreground Bash
  calls that took up to 3.6 s has one, each of the 14 that took 4 s or more has one), then nothing until the first
  `tool_progress` at 30 s:
  `{"type":"tool_progress","tool_use_id":"<call id>-heartbeat-N","tool_name":"Bash","parent_tool_use_id":"<call id>",
  "elapsed_time_seconds":30|60|90,"heartbeat":true}`, one every 30 s. The call's own id is in `parent_tool_use_id`,
  not in `tool_use_id`. A 20 s call gets no `tool_progress` at all. The command's output is in none of these lines.
- Live output is available on request: `get_task_output` with the `task_id` of `task_started` answered the tail of
  the running foreground command (`"tick 1\ntick 2\ntick 3\n"`).
- Interrupt mid-tool: the response came after 0.003 s; `task_notification` with `status:"stopped"`, the rejected
  `tool_result`, the `[Request interrupted by user for tool use]` line and the error `result` all carry the same time
  stamp (within 0.01 s of the request; the driver's own polling saw the result after 0.2 s). The `sleep 21` process was
  gone 1 s later.
- Exit code after stdin close was 1 in this run, where the last `result` was the interrupted (error) one; it was 0
  in every run whose last `result` was a success (n=1 for the error case).

## C. Modes, model, plan

### 9. `--permission-mode acceptEdits`: `mv`, `rm`, and an edit of `.claude/settings.local.json`

Question: do `rm` / `mv` in Bash ask (`can_use_tool`) or run? Does an Edit of `.claude/settings.local.json` inside the work dir ask?

Fixture: `r4-accept-edits`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash,Read,Edit,Write`. The work dir has a `.claude/settings.local.json` holding `{}` before the start.

```json
{"type":"user","session_id":"","message":{"role":"user","content":"Use the Write tool to create the file note.txt with the content: hi"},"parent_tool_use_id":null}
{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command: mv hello.txt hello2.txt"},"parent_tool_use_id":null}
{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command: rm hello2.txt"},"parent_tool_use_id":null}
{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command: rm -rf sub && mkdir sub && mv note.txt sub/a && mv sub/a sub/b"},"parent_tool_use_id":null}
{"type":"user","session_id":"","message":{"role":"user","content":"Use the Read tool on .claude/settings.local.json, then use the Edit tool to replace `{}` with `{ }` in that file."},"parent_tool_use_id":null}
```

Tool calls, results and requests (the driver's note after each turn counts the `can_use_tool` requests of that turn):

```json
# t=1.30
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"Write","input":{"file_path":"/tmp/work/r4-accept-edits/note.txt","content":"hi"}}],"…":"…"},"…":"…"}
# t=1.30
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"File created successfully at: /tmp/work/r4-accept-edits/note.txt (file state is current in your context — no need to Read it back)"}]},"tool_use_result":{"type":"create","filePath":"/tmp/work/r4-accept-edits/note.txt","content":"hi","structuredPatch":[],"originalFile":null,"userModified":false},"tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"config","reason_type":"mode"}}],"…":"…"}
# t=2.27
{"_meta":{"turn":1,"can_use_tool_requests":0},"_t":2.271}
# t=2.80
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-2","name":"Bash","input":{"command":"mv hello.txt hello2.txt","description":"Rename hello.txt to hello2.txt"}}],"…":"…"},"…":"…"}
# t=3.93
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-2","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":true},"tool_result_meta":[{"id":"toolu-2","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=4.95
{"_meta":{"turn":2,"can_use_tool_requests":0},"_t":4.948}
# t=6.37
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-3","name":"Bash","input":{"command":"ls -l hello2.txt && cat hello2.txt","description":"Inspect hello2.txt before deleting"}}],"…":"…"},"…":"…"}
# t=6.37
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-3","type":"tool_result","content":".rw-r--r--@ 6 me 1 Jan 00:00 hello2.txt\nhello","is_error":false}]},"tool_use_result":{"stdout":".rw-r--r--@ 6 me 1 Jan 00:00 hello2.txt\nhello","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false},"tool_result_meta":[{"id":"toolu-3","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=7.33
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-4","name":"Bash","input":{"command":"rm hello2.txt","description":"Delete hello2.txt"}}],"…":"…"},"…":"…"}
# t=7.59
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-4","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":true},"tool_result_meta":[{"id":"toolu-4","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=9.06
{"_meta":{"turn":3,"can_use_tool_requests":0},"_t":9.064}
# t=10.34
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-5","name":"Bash","input":{"command":"pwd && ls -la sub 2>&1; ls -l note.txt 2>&1","description":"List sub directory contents and check note.txt"}}],"…":"…"},"…":"…"}
# t=10.34
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-5","type":"tool_result","content":"/tmp/work/r4-accept-edits\n\"sub\": No such file or directory (os error 2)\n.rw-r--r--@ 2 me 1 Jan 00:00 note.txt","is_error":false}]},"tool_use_result":{"stdout":"/tmp/work/r4-accept-edits\n\"sub\": No such file or directory (os error 2)\n.rw-r--r--@ 2 me 1 Jan 00:00 note.txt","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false},"tool_result_meta":[{"id":"toolu-5","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=11.49
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-6","name":"Bash","input":{"command":"rm -rf sub && mkdir sub && mv note.txt sub/a && mv sub/a sub/b","description":"Recreate sub dir and move note.txt into it as b"}}],"…":"…"},"…":"…"}
# t=11.49
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-6","type":"tool_result","content":"(Bash completed with no output)","is_error":false}]},"tool_use_result":{"stdout":"","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":true},"tool_result_meta":[{"id":"toolu-6","permission_decision":{"decision":"accept","source":"config","reason_type":"subcommandResults"}}],"…":"…"}
# t=12.73
{"_meta":{"turn":4,"can_use_tool_requests":0},"_t":12.735}
# t=13.54
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-7","name":"Read","input":{"file_path":"/tmp/work/r4-accept-edits/.claude/settings.local.json"}}],"…":"…"},"…":"…"}
# t=13.54
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-7","type":"tool_result","content":"1\t{}\n2\t"}]},"tool_use_result":{"type":"text","file":{"filePath":"/tmp/work/r4-accept-edits/.claude/settings.local.json","content":"{}\n","numLines":2,"startLine":1,"totalLines":2}},"tool_result_meta":[{"id":"toolu-7","permission_decision":{"decision":"accept","source":"config","reason_type":"mode"}}],"…":"…"}
# t=14.77
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-8","name":"Edit","input":{"replace_all":false,"file_path":"/tmp/work/r4-accept-edits/.claude/settings.local.json","old_string":"{}","new_string":"{ }"}}],"…":"…"},"…":"…"}
# t=14.77
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Edit","display_name":"Edit","input":{"file_path":"/tmp/work/r4-accept-edits/.claude/settings.local.json","old_string":"{}","new_string":"{ }","replace_all":false},"description":".claude/settings.local.json","permission_suggestions":[],"decision_reason":"Claude requested permissions to write to /tmp/work/r4-accept-edits/.claude/settings.local.json, but you haven't granted it yet.","decision_reason_type":"safetyCheck","classifier_approvable":true,"tool_use_id":"toolu-8"}}
# t=14.78
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-8","type":"tool_result","content":"The file /tmp/work/r4-accept-edits/.claude/settings.local.json has been updated successfully. (file state is current in your context — no need to Read it back)"}]},"tool_use_result":{"filePath":"/tmp/work/r4-accept-edits/.claude/settings.local.json","oldString":"{}","newString":"{ }","originalFile":"{}\n","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":1,"lines":["-{}","+{ }"]}],"userModified":false,"replaceAll":false},"tool_result_meta":[{"id":"toolu-8","permission_decision":{"decision":"accept","source":"user_temporary"}}],"…":"…"}
# t=15.58
{"_meta":{"turn":5,"can_use_tool_requests":1},"_t":15.581}
# t=15.58
{"_meta":{"files":[".claude",".git","…(2 more)"],"settings_local":"{ }\n"},"_t":15.581}
```

What the adapter can rely on:
- In `acceptEdits` the Write tool and the Bash commands `mv`, `rm`, `rm -rf sub && mkdir sub && mv … && mv …` on
  paths inside the work dir ran with NO `can_use_tool` request (`permission_decision` `source:"config"`, `reason_type`
  `mode` for the file tools and `subcommandResults` for Bash). `rm` of a file that git tracks included.
- An Edit of `.claude/settings.local.json` DOES ask, in the same mode: `can_use_tool` with
  `"decision_reason_type":"safetyCheck"`, a `decision_reason` text, `"classifier_approvable":true` and an empty
  `permission_suggestions`. After the allow the file was changed.

### 10. Plan mode set on a session started as the app starts it; tool names that do not exist

Question: is `ExitPlanMode` usable after `set_permission_mode` plan on a session started in the default mode with the app's tool list? What does the CLI say about a tool name that does not exist?

Fixtures: `r4-plan-from-default`, `r4-unknown-tool-names`.

`r4-plan-from-default` flags: `--permission-mode manual --strict-mcp-config --setting-sources local --tools Bash,Read,Edit,Write,Grep,Glob,TaskCreate,TaskUpdate,TaskList,TaskGet,WebFetch,WebSearch,Agent,Monitor,TaskStop,Skill,LSP,ExitPlanMode` (the `--tools` list of `tools_for()` in `claude.rs` for a 1:1 agent with a shell).

`initialize` says `"current_permission_mode":"default"` for `--permission-mode manual`. Then:

```json
{"type":"control_request","request_id":"host-2","request":{"subtype":"set_permission_mode","mode":"plan"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"mode":"plan"}}}
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"plan","tools":["Task","Bash","Edit","ExitPlanMode","Glob","Grep","Monitor","Read","Skill","TaskCreate","TaskGet","TaskList","TaskStop","TaskUpdate","WebSearch","Write"],"mcp_servers":[],"…":"…"}
```

`init.tools` has 16 names for the 18 asked: `Agent` is listed as `Task`; `WebFetch` and `LSP` are not there. No line says why.

The models of the assistant lines in this one turn (same `--model haiku` process):

```text
-- system/status permissionMode=plan
claude-sonnet-5-5  tool_use Bash
claude-sonnet-5-5  thinking
claude-sonnet-5-5  tool_use Write
claude-sonnet-5-5  tool_use ExitPlanMode
claude-sonnet-5-5  tool_use Write
claude-sonnet-5-5  tool_use ExitPlanMode
-- system/status permissionMode=default
claude-haiku-5-5  thinking
claude-haiku-5-5  text
claude-haiku-5-5  tool_use Read
claude-haiku-5-5  tool_use Edit
claude-haiku-5-5  tool_use Read
claude-haiku-5-5  text
```

`r4-unknown-tool-names` flags: `--permission-mode manual --strict-mcp-config --setting-sources local --tools Bash,Read,NoSuchToolXyz,LSP,Monitor,TaskStop,Skill --allowedTools 'NoSuchToolAbc,mcp__nosuch__*,Bash(echo *)'`. stderr was empty. The init line, and the rules the unknown `--allowedTools` names became:

```json
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","Monitor","Read","Skill","TaskStop"],"mcp_servers":[],"…":"…"}
{"behavior":"allow","source":"cliArg","rule":"NoSuchToolAbc","description":{"prefix":"Any use of the ","emphasis":"NoSuchToolAbc","suffix":" tool"},"editability":"session"}
{"behavior":"allow","source":"cliArg","rule":"mcp__nosuch__*","description":{"prefix":"Any use of the ","emphasis":"mcp__nosuch__*","suffix":" tool"},"editability":"session"}
{"behavior":"allow","source":"cliArg","rule":"Bash(echo *)","description":{"prefix":"Any Bash command starting with ","emphasis":"echo"},"editability":"session"}
```

What the adapter can rely on:
- `--permission-mode manual` is accepted and is reported as `default` (`initialize.current_permission_mode`,
  `init.permissionMode`).
- `set_permission_mode` `plan` before the first message works; with `ExitPlanMode` in `--tools` the plan arrives as a
  `can_use_tool` request (scenario 11). Usable: yes.
- While the session is in plan mode, the model requests of a `--model haiku` session ran on `claude-sonnet-5-5`
  (`assistant.message.model`), and went back to `claude-haiku-5-5` after the plan was accepted. `init.model` said
  `claude-haiku-5-5` all the time. `result.modelUsage` has both models; 95 % of this turn's cost was the sonnet part.
  The plan-mode recording of part 1 shows the same. The model a turn runs on is `assistant.message.model`, not
  `init.model`.
- A tool name that does not exist in `--tools` is dropped without a word: nothing on stderr, no error line, the
  session runs. The only sign is that `init.tools` does not have it. A known tool that is not available here (`LSP`,
  and `WebFetch` in `r4-plan-from-default`) is missing in the same silent way. The text "did not enable requested
  tools" is the adapter's own; the CLI bundle has no such text.
- An unknown name in `--allowedTools` becomes an allow rule of `source:"cliArg"` (`list_permission_rules`), also
  without a word.

### 11. `ExitPlanMode` answered with deny + message, and with allow + a changed plan

Question: what does the CLI do with a changed plan in `updatedInput`? And with deny + message?

Fixture: `r4-plan-from-default` (same run as scenario 10).

```json
{"type":"user","session_id":"","message":{"role":"user","content":"Plan a tiny change: add a second line `world` to hello.txt. Keep the plan to 2-3 short steps and present it with ExitPlanMode. When the plan is approved, carry out the approved plan."},"parent_tool_use_id":null}
```

```json
# t=5.55
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","display_name":"ExitPlanMode","input":{"plan":"# Plan: add `world` to hello.txt\n\n## Context\nhello.txt currently contains one line, `hello`. Add a second line, `world`.\n\n## Steps\n1. Append `world` on a new line to hello.txt (keep the trailing newline).\n2. Verify with `cat hello.txt`: it should show `hello` then `world`.\n","planFilePath":"/Users/me/.claude/plans/plan-a-tiny-change-silly-octopus.md"},"tool_use_id":"toolu-3","requires_user_interaction":true}}
# t=5.55
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"deny","message":"Not yet: add a final step that reads the file back to check it. Then present the plan again.","interrupt":false}}},"_t":5.555}
# t=5.55
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Not yet: add a final step that reads the file back to check it. Then present the plan again.","is_error":true,"tool_use_id":"toolu-3"}]},"tool_use_result":"Error: Not yet: add a final step that reads the file back to check it. Then present the plan again.","tool_result_meta":[{"id":"toolu-3","non_execution_kind":"permission-rule","permission_decision":{"decision":"reject","source":"user_reject"}}],"…":"…"}
# t=8.10
{"type":"control_request","request_id":"req-2","request":{"subtype":"can_use_tool","tool_name":"ExitPlanMode","display_name":"ExitPlanMode","input":{"plan":"# Plan: add `world` to hello.txt\n\n## Context\nhello.txt currently contains one line, `hello`. Add a second line, `world`.\n\n## Steps\n1. Append `world` on a new line to hello.txt (keep the trailing newline).\n2. Verify with `cat hello.txt`: it should show `hello` then `world`.\n3. Read the file back with the Read tool to confirm the final contents are exactly two lines: `hello`, `world`.\n","planFilePath":"/Users/me/.claude/plans/plan-a-tiny-change-silly-octopus.md"},"tool_use_id":"toolu-5","requires_user_interaction":true}}
# t=8.12
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-2","response":{"behavior":"allow","updatedInput":{"plan":"# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n","planFilePath":"/Users/me/.claude/plans/plan-a-tiny-change-silly-octopus.md"}}}},"_t":8.116}
# t=8.12
{"type":"system","subtype":"status","status":null,"permissionMode":"default"}
# t=8.12
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"User has approved your plan. You can now start coding. Start with updating your todo list if applicable\n\nYour plan has been saved to: /Users/me/.claude/plans/plan-a-tiny-change-silly-octopus.md\nYou can refer back to it if needed during implementation.\n\n## Approved Plan (edited by user):\n# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n","tool_use_id":"toolu-5"}]},"tool_use_result":{"plan":"# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n","isAgent":false,"filePath":"/Users/me/.claude/plans/plan-a-tiny-change-silly-octopus.md","planWasEdited":true},"tool_result_meta":[{"id":"toolu-5","permission_decision":{"decision":"accept","source":"user_temporary"}}],"…":"…"}
# t=11.89
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-3","response":{"behavior":"allow","updatedInput":{"file_path":"/tmp/work/r4-plan-from-default/hello.txt","old_string":"hello\n","new_string":"hello\nplanet\n","replace_all":false}}}},"_t":11.893}
```

After the turn: `get_plan`, and the driver's look at the files:

```json
{"type":"control_request","request_id":"host-3","request":{"subtype":"get_plan"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"exists":true,"content":"# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n","path":"/Users/me/.claude/plans/plan-a-tiny-change-silly-octopus.md"}}}
{"_meta":{"hello_txt":"hello\nplanet\n","plan_file_exists":true,"plan_file_content":"# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n","exit_plan_mode_requests":2},"_t":13.295}
```

What the adapter can rely on:
- Both requests have `"requires_user_interaction":true` and no `permission_suggestions`.
- Deny + message: the `tool_result` is the message with `is_error:true`; the session stays in plan mode; the model
  changed its plan file and called `ExitPlanMode` again.
- Allow + changed `updatedInput.plan`: the CLI takes the changed text as the plan. `system/status` switches to
  `permissionMode:"default"`. The `tool_result` text says `## Approved Plan (edited by user):` followed by the changed
  text (an unchanged plan says `## Approved Plan:`, part 1). The plan FILE on disk is rewritten with the changed text
  (`get_plan.content`, and the file itself), and `tool_use_result` has `"planWasEdited":true`. The model then did
  what the changed plan says (`planet`, not `world`) and said so.
- After the approval the next Edit asked through `can_use_tool` (mode `default`).

### 12. `/compact` on a session with almost no history

Question: is there a `result`? What lines come?

Fixture: `r4-compact-empty`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash`. Sent: `/compact` as the very first message, one tiny turn, `/compact` again.

`/compact` first (every line):

```json
# t=0.00
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"/compact"},"parent_tool_use_id":null},"_t":0.004}
# t=0.47
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=0.47
{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Error: No messages to compact"}],"…":"…"},"…":"…"}
# t=0.47
{"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","stop_reason":null,"local_command":"compact","queued_turn_count":0,"result_index":0,"…":"…"}
```

`/compact` after one turn (every line but the summary text, which is cut):

```json
# t=4.25
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":"/compact"},"parent_tool_use_id":null},"_t":4.248}
# t=4.25
{"type":"system","subtype":"status","status":"compacting"}
# t=8.86
{"type":"system","subtype":"status","status":null,"compact_result":"success"}
# t=8.86
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=8.86
{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"manual","pre_tokens":5765,"post_tokens":934,"cumulative_dropped_tokens":4831,"duration_ms":4604},"logical_parent_uuid":"uuid-20"}
# t=8.86
{"type":"user","message":{"role":"user","content":"This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\n<system…"},"isReplay":false,"isSynthetic":true,"…":"…"}
# t=8.86
{"type":"user","message":{"role":"user","content":"<local-command-stdout>Compacted </local-command-stdout>"},"isReplay":true,"…":"…"}
# t=8.86
{"type":"result","subtype":"success","is_error":false,"num_turns":0,"result":"","stop_reason":null,"local_command":"compact","queued_turn_count":0,"result_index":2,"…":"…"}
```

What the adapter can rely on:
- Yes, always a `result`. With nothing to compact: `system/init`, one `assistant` line with `"model":"<synthetic>"`
  and the text `Error: No messages to compact`, then a `result` with `subtype:"success"`, `is_error:false`,
  `num_turns` 0, empty `result` text, `local_command:"compact"`, cost 0. No `status` `compacting`, no
  `compact_boundary`. The "error" is only in that text.
- After one tiny turn `/compact` runs in full (4.6 s): `status` `compacting`, `status` with
  `compact_result:"success"`, `system/init`, `compact_boundary` (`pre_tokens` 5765, `post_tokens` 934), the summary
  `user` line, the replayed `<local-command-stdout>Compacted </local-command-stdout>` line, the `result`.

### 13. `set_model` "default", `apply_flag_settings` with an effort level, `--fallback-model`

Fixture: `r4-model-effort-fallback`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --fallback-model sonnet`. No turn ran on another model than haiku.

```json
{"type":"control_request","request_id":"host-3","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-3","response":{"effective":{},"sources":[],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-4","request":{"subtype":"set_model","model":"default"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"model":"claude-opus-5-5"}}}
{"type":"control_request","request_id":"host-5","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-5","response":{"effective":{},"sources":[],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-opus-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-6","request":{"subtype":"set_model","model":"haiku"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-6","response":{"model":"claude-haiku-5-5"}}}
{"type":"control_request","request_id":"host-7","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"low"}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-7"}}
{"type":"control_request","request_id":"host-8","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-8","response":{"effective":{"effortLevel":"low"},"sources":[{"source":"flagSettings","settings":{"effortLevel":"low"}}],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"low","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-9","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"not-a-level"}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-9"}}
{"type":"control_request","request_id":"host-10","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-10","response":{"effective":{},"sources":[{"source":"flagSettings","settings":{}}],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"low","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-11","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"high"}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-11"}}
{"type":"control_request","request_id":"host-12","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-12","response":{"effective":{"effortLevel":"high"},"sources":[{"source":"flagSettings","settings":{"effortLevel":"high"}}],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"high","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-13","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-13","response":{"effective":{"effortLevel":"high"},"sources":[{"source":"flagSettings","settings":{"effortLevel":"high"}}],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"high","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-14","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":null}}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-14"}}
{"type":"control_request","request_id":"host-15","request":{"subtype":"get_settings"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-15","response":{"effective":{},"sources":[],"remote_control_policy_lock_reason":"<text>","applied":{"model":"claude-haiku-5-5","effort":"medium","advisor":null,"ultracode":false,"ultracodeRequested":false,"ultracodeAvailable":true}}}}
{"type":"control_request","request_id":"host-16","request":{"subtype":"set_model","model":"default"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-16","response":{"model":"claude-opus-5-5"}}}
{"type":"control_request","request_id":"host-17","request":{"subtype":"set_model","model":"haiku"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-17","response":{"model":"claude-haiku-5-5"}}}
```

What the adapter can rely on:
- `set_model` with `"default"` works and answers the model the account's default resolves to
  (`{"model":"claude-opus-5-5"}` here); `get_settings.applied.model` follows. Before the first message nothing else is
  written.
- `apply_flag_settings` `{"settings":{"effortLevel":"low"|"high"}}` answers success with no inner `response`. The
  effect shows in `get_settings`: `effective.effortLevel`, a `sources` entry of `"source":"flagSettings"`, and
  `applied.effort`. `{"effortLevel":null}` removes it again (`applied.effort` back to `medium`).
- A bad level (`"not-a-level"`) is ALSO answered with success. It is dropped (`effective` is `{}`), and
  `applied.effort` stays at the value before it (`low`). The host must read `get_settings` to know.
- `--fallback-model sonnet` shows nowhere: not in `system/init`, not in the `initialize` answer, not in `get_status`,
  not in `get_settings`. (The only `fallback` strings of the fixture are the flag in `_argv` and a
  `"fallback_credit":null` inside `result.usage`.) No fallback was triggered.

## D. Sandbox

Settings are passed as in part 1, section 9: `--settings '{"sandbox":{…}}'`.

### 14. Sandbox on + `curl -sI https://example.com`

Question: the exact request the host gets, and what follows on allow and on deny.

Fixtures: `r4-sandbox-network-allow`, `r4-sandbox-network-deny`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true}}'`.

Allow:

```json
# t=1.75
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"Bash","input":{"command":"curl -sI https://example.com","description":"Fetch response headers from example.com"}}],"…":"…"},"…":"…"}
# t=5.22
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"SandboxNetworkAccess","display_name":"SandboxNetworkAccess","input":{"host":"example.com"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"WebFetch","ruleContent":"domain:example.com"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"uuid-16","description":"Allow network connection to example.com?"}}
# t=5.22
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"allow","updatedInput":{"host":"example.com"}}}},"_t":5.216}
# t=5.22
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":"HTTP/1.1 200 Connection Established\r\n\r\nHTTP/2 200 \r\ndate: Mon, 23 Nov 2026 01:12:27 GMT\r\ncontent-type: text/html; charset=utf-8\r\nserver: cloudflare\r\nlast-modified: Thu, 19 Nov 2026 22:11:52 GMT\r\nallow: GET, HEAD\r\naccept-ranges: bytes\r\nage: 3\r\ncf-cache-status: HIT\r\ncf-ray: 0000000000000000-XXX\r\nalt-svc: h3=\":443\"; ma=86400","is_error":false}]},"tool_use_result":{"stdout":"HTTP/1.1 200 Connection Established\r\n\r\nHTTP/2 200 \r\ndate: Mon, 23 Nov 2026 01:12:27 GMT\r\ncontent-type: text/html; charset=utf-8\r\nserver: cloudflare\r\nlast-modified: Thu, 19 Nov 2026 22:11:52 GMT\r\nallow: GET, HEAD\r\naccept-ranges: bytes\r\nage: 3\r\ncf-cache-status: HIT\r\ncf-ray: 0000000000000000-XXX\r\nalt-svc: h3=\":443\"; ma=86400","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false},"tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"config","reason_type":"other"}}],"…":"…"}
```

Deny:

```json
# t=1.28
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"toolu-1","name":"Bash","input":{"command":"curl -sI https://example.com","description":"Fetch response headers from example.com"}}],"…":"…"},"…":"…"}
# t=2.82
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"SandboxNetworkAccess","display_name":"SandboxNetworkAccess","input":{"host":"example.com"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"WebFetch","ruleContent":"domain:example.com"}],"behavior":"allow","destination":"localSettings"}],"tool_use_id":"uuid-16","description":"Allow network connection to example.com?"}}
# t=2.82
{"_sent":{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"deny","message":"The user denied this action.","interrupt":false}}},"_t":2.822}
# t=2.82
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"Exit code 56\nHTTP/1.1 403 user denied\r\nContent-Type: text/plain\r\nX-Proxy-Error: blocked-by-allowlist\r\n\r\n\n<sandbox_violations>\ndeny network-outbound example.com:443 (user denied)\n</sandbox_violations>","is_error":true,"tool_use_id":"toolu-1"}]},"tool_use_result":"Error: Exit code 56\nHTTP/1.1 403 user denied\r\nContent-Type: text/plain\r\nX-Proxy-Error: blocked-by-allowlist\r\n\r\n\n<sandbox_violations>\ndeny network-outbound example.com:443 (user denied)\n</sandbox_violations>","tool_result_meta":[{"id":"toolu-1","remedy":{"kind":"sandbox_violation"},"permission_decision":{"decision":"accept","source":"config","reason_type":"other"}}],"…":"…"}
```

What the adapter can rely on:
- The Bash call itself sends no request (sandbox auto-allow). While it runs, the sandbox's proxy asks the host:
  `can_use_tool` with `"tool_name":"SandboxNetworkAccess"`, `"display_name":"SandboxNetworkAccess"`,
  `"input":{"host":"example.com"}`, `"description":"Allow network connection to example.com?"`, one
  `permission_suggestions` entry (`addRules`, rule `{"toolName":"WebFetch","ruleContent":"domain:example.com"}`,
  `behavior:"allow"`, `destination:"localSettings"`), and a `tool_use_id` that is a uuid of its own, NOT the id of the
  Bash call. There is no `decision_reason_type`. It came 3.3 s (allow run) and 1.3 s (deny run) after the `tool_use`.
- Allow (`{"behavior":"allow","updatedInput":{"host":…}}`): the command goes on and its result is normal. The output
  starts with the proxy's `HTTP/1.1 200 Connection Established`: the traffic goes through the sandbox's proxy.
- Deny: the Bash `tool_result` has `is_error:true`, text `Exit code 56`, the proxy's `HTTP/1.1 403 user denied`,
  `X-Proxy-Error: blocked-by-allowlist`, and a `<sandbox_violations>` block
  (`deny network-outbound example.com:443 (user denied)`); `tool_result_meta` has `"remedy":{"kind":"sandbox_violation"}`.
  The turn goes on normally.
- After the allow, `list_permission_rules` shows no new rule and `get_sandbox_dialog.restrictions.network_allowed_domains`
  is still `[]` (the allow was sent without `updatedPermissions`).

### 15. `autoAllowBashIfSandboxed:false`, and the app's `manual` mode

Question: do sandboxed Bash commands then ask through `can_use_tool`?

Fixtures: `r4-sandbox-noauto` (`--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true, "autoAllowBashIfSandboxed": false}}'`), `r4-sandbox-manual` (`--permission-mode manual --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true}}'`), `r4-sandbox-manual-noauto` (`--permission-mode manual --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true, "autoAllowBashIfSandboxed": false}}'`).

Three turns each: `echo hi > inside.txt`, `ls`, `mv inside.txt inside2.txt`. Requests per turn (driver notes) and the `mode` of `get_sandbox_dialog`:

| fixture | permission mode | `autoAllowBashIfSandboxed` | `get_sandbox_dialog.mode` | `echo hi > inside.txt` | `ls` | `mv` |
|---|---|---|---|---|---|---|
| `r4-sandbox-noauto` | `acceptEdits` | false | `regular` | runs | runs | runs |
| `r4-sandbox-manual` | `manual` | (default) | `auto-allow` | runs | runs | runs |
| `r4-sandbox-manual-noauto` | `manual` | false | `regular` | asks | runs | asks |

The request for `echo hi > inside.txt` in `r4-sandbox-manual-noauto`:

```json
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"echo hi > inside.txt","description":"Write \"hi\" into inside.txt in the working directory"},"description":"Write \"hi\" into inside.txt in the working directory","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"echo hi *"}],"behavior":"allow","destination":"localSettings"},{"type":"addDirectories","directories":["/tmp/work/r4-sandbox-manual-noauto"],"destination":"session"}],"blocked_path":"/tmp/work/r4-sandbox-manual-noauto/inside.txt","tool_use_id":"toolu-1"}}
{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"allow","updatedInput":{"command":"echo hi > inside.txt","description":"Write \"hi\" into inside.txt in the working directory"}}}}
```

What the adapter can rely on:
- With the sandbox on and nothing else (auto-allow, `mode:"auto-allow"`), Bash commands do not ask, also when the
  session's permission mode is `manual`/`default`. Part 1 showed this for `acceptEdits` only.
- `autoAllowBashIfSandboxed:false` (`mode:"regular"`) puts Bash back under the permission mode: in `manual` the two
  writing commands asked with an ordinary `can_use_tool` (suggestions included, no sandbox marker in the request) and
  `ls` ran; in `acceptEdits` nothing asked, as without a sandbox (scenario 9). `get_sandbox_dialog` still says
  `enabled:true` in these runs.

### 16. `failIfUnavailable`, and how the host can see that the sandbox is on

Fixtures: `r4-sandbox-required-available` (`--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true, "failIfUnavailable": true}}'`), `r4-sandbox-required-unavailable` (`--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true, "failIfUnavailable": true, "enabledPlatforms": ["linux"]}}'`).

`get_sandbox_dialog` with `failIfUnavailable:true` on a machine where the sandbox works:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"get_sandbox_dialog"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"supported":true,"locked":true,"overrides_locked":true,"enabled":true,"enabled_in_settings":true,"mode":"auto-allow","auto_allow_available":true,"no_sandbox_allowed":true,"unsandboxed_fallback":true,"dependencies":{"errors":[],"warnings":[]},"excluded_commands":[],"restrictions":{"fs_deny_read":[],"fs_allow_read":[],"fs_allow_write":[],"fs_deny_write":[],"network_allowed_domains":[],"network_denied_domains":[],"network_managed":false,"unix_sockets":[],"allow_all_unix_sockets":false,"ignored_glob_patterns":[]}}}}
```

What the adapter can rely on:
- With `failIfUnavailable:true` and a working sandbox nothing differs: the session starts, Bash runs sandboxed, and
  `get_sandbox_dialog` has no field for the setting.
- The control request that reports the sandbox is `get_sandbox_dialog` (read-only, answered before the first
  message): `supported`, `enabled` (running in this session), `enabled_in_settings`, `mode` (`auto-allow`, `regular`,
  `disabled`), `unsandboxed_fallback`, `dependencies.errors` / `.warnings`, `restrictions`. `system/init` has no
  sandbox key.
- NOT verified: what the CLI prints when the sandbox is required and cannot start. The one harmless way tried, adding
  `"enabledPlatforms":["linux"]` in `--settings` on macOS, has no effect (`r4-sandbox-required-unavailable`: still
  `supported:true`, `enabled:true`, the command ran): the bundle calls it an enterprise (managed) setting. Bundle, not
  recorded: the schema text of `failIfUnavailable` is "Exit with an error at startup if sandbox.enabled is true but
  the sandbox cannot start … When false (default), a warning is shown and commands run unsandboxed"; a stderr text
  "Set sandbox.failIfUnavailable=false to allow unsandboxed execution" exists; and a tool error for a sandbox that is
  lost mid-session ("Sandbox is required (sandbox.failIfUnavailable) and stopped being available in this session").

### 17. Sandbox on, in a linked git worktree: can `git commit` write to the main repo's `.git`?

Fixtures: `r4-sandbox-worktree-linked` (sandbox on, cwd = a `git worktree add` folder), `r4-sandbox-worktree-plain` (sandbox on, ordinary repo), `r4-sandbox-worktree-linked-off` (no sandbox). Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --settings '{"sandbox": {"enabled": true}}'`.

Sent, and the driver's notes (`r4-sandbox-worktree-linked`):

```json
{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command once (do not retry it, do not change it) and report its output: echo wt > wt.txt && git add wt.txt && git -c core.hooksPath=/dev/null -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false commit -m wt"},"parent_tool_use_id":null}
{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command once (do not retry it, do not change it) and report its output: echo probe > ../r4-wt-main-linked/outside-probe.txt"},"parent_tool_use_id":null}
{"type":"user","session_id":"","message":{"role":"user","content":"Run exactly this Bash command once (do not retry it, do not change it) and report its output: echo probe > ../r4-wt-unrelated-linked/outside-probe.txt"},"parent_tool_use_id":null}
{"_meta":{"git_dir":"/tmp/work/r4-wt-main-linked/.git/worktrees/r4-wt-linked-linked","git_common_dir":"/tmp/work/r4-wt-main-linked/.git","commits_before":"1"},"_t":0.055}
{"_meta":{"outside_probe_written":true,"unrelated_folder_probe_written":true},"_t":10.24}
{"_meta":{"commits_after":"2","commit_was_written":true,"can_use_tool_requests":[["Bash","subcommandResults",false]]},"_t":10.24}
```

The request for the commit command (the same in all three fixtures), and the line the commit caused:

```json
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"echo wt > wt.txt && git add wt.txt && git -c core.hooksPath=/dev/null -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false commit -m wt","description":"Write wt.txt, stage it, and commit with hooks disabled"},"description":"Write wt.txt, stage it, and commit with hooks disabled","permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"git add *"},{"toolName":"Bash","ruleContent":"git -c core.hooksPath=/dev/null -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false commit -m wt"}],"behavior":"allow","destination":"localSettings"}],"decision_reason_type":"subcommandResults","tool_use_id":"toolu-1"}}
{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"allow","updatedInput":{"command":"echo wt > wt.txt && git add wt.txt && git -c core.hooksPath=/dev/null -c user.name=demo -c user.email=demo@example.invalid -c commit.gpgsign=false commit -m wt","description":"Write wt.txt, stage it, and commit with hooks disabled"}}}}
{"type":"system","subtype":"vcs_state_changed","kind":"commit","branch":"wt-linked","cwd":"/tmp/work/r4-wt-linked-linked"}
```

Answer: NOT verified. The commit was written (2 commits after, 1 before), but the recording cannot say that the
sandbox allowed it for the worktree's sake:
- The task fixed the work folders under the session's scratch folder, which lies under the CLI's own temp folder
  (`/tmp/claude-<uid>/`, shown as `/tmp/claude-tmp/` and `/tmp/work`). The sandbox lets commands write there: the
  control turns wrote a file into the main repo's working tree AND into an unrelated sibling folder with no request and
  no error (`outside_probe_written`, `unrelated_folder_probe_written` both true). In part 1 a write to `/tmp/<file>`
  outside that tree was refused, so the sandbox was on; the folders of this scenario are simply inside a tree it allows.
- To answer it, run `r4_d17_sandbox_worktree.py` with `CAPTURE_DIR` set to a folder outside `/tmp/claude-<uid>/`.

What the adapter can rely on (recorded here, independent of that):
- With the sandbox on (auto-allow) a `git add … && git … commit` command still ASKS: `can_use_tool` with
  `"decision_reason_type":"subcommandResults"` and suggestions for `git add *` and for the exact commit command. The
  same request comes with the sandbox off. So "sandbox on" does not mean "no Bash request ever".
- A commit writes a `system/vcs_state_changed` line (`kind:"commit"`, `branch`, `cwd`): a subtype part 1 did not have.
- Bundle, not recorded: the model's own instructions say that when the CLI was started in a SUBFOLDER of a repo,
  sandboxed commands cannot write the `.git` folder above the working directory (`git add`/`commit` fail with
  "Unable to create … .lock … Permission denied"), and that the sandbox write-protects the `commondir` and
  `config.worktree` files of linked worktrees.

## E. MCP and project files

### 18. An MCP server that needs a sign-in

Question: what do `system/init.mcp_servers` and `mcp_status` say? `mcp_reconnect` and `mcp_toggle` shapes.

Fixture: `r4-mcp-needs-auth`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools Bash --mcp-config /tmp/work/r4-mcp-needs-auth.json`, with this file:

```json
{"mcpServers":{"needs-login":{"type":"http","url":"http://127.0.0.1:48731/mcp"}}}
```

The server is a python3 `http.server` in the driver: every request gets `401` with `WWW-Authenticate: Bearer realm="needs-login", error="invalid_token", error_description="Sign in first"`; paths under `/.well-known/` get `404`.

Every control request with its answer, and the init line:

```json
{"type":"control_request","request_id":"host-1","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-1","response":{"mcpServers":[{"name":"needs-login","status":"pending","config":{"type":"http","url":"http://127.0.0.1:48731/mcp"},"transport":"http","scope":"dynamic","source":"dynamic"}]}}}
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"acceptEdits","tools":["Bash"],"mcp_servers":[{"name":"needs-login","status":"failed","source":"dynamic"}],"…":"…"}
{"type":"control_request","request_id":"host-2","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-2","response":{"mcpServers":[{"name":"needs-login","status":"failed","error":"Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}","config":{"type":"http","url":"http://127.0.0.1:48731/mcp"},"transport":"http","scope":"dynamic","source":"dynamic"}]}}}
{"type":"control_request","request_id":"host-3","request":{"subtype":"mcp_reconnect","serverName":"needs-login"}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-3","error":"Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}"}}
{"type":"control_request","request_id":"host-4","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-4","response":{"mcpServers":[{"name":"needs-login","status":"failed","error":"Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}","config":{"type":"http","url":"http://127.0.0.1:48731/mcp"},"transport":"http","scope":"dynamic","source":"dynamic"}]}}}
{"type":"control_request","request_id":"host-5","request":{"subtype":"mcp_toggle","serverName":"needs-login","enabled":false}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-5"}}
{"type":"control_request","request_id":"host-6","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-6","response":{"mcpServers":[{"name":"needs-login","status":"disabled","config":{"type":"http","url":"http://127.0.0.1:48731/mcp"},"transport":"http","scope":"dynamic","source":"dynamic"}]}}}
{"type":"control_request","request_id":"host-7","request":{"subtype":"mcp_toggle","serverName":"needs-login","enabled":true}}
{"type":"control_response","response":{"subtype":"error","request_id":"host-7","error":"Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}"}}
{"type":"control_request","request_id":"host-8","request":{"subtype":"mcp_status"}}
{"type":"control_response","response":{"subtype":"success","request_id":"host-8","response":{"mcpServers":[{"name":"needs-login","status":"failed","error":"Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}","config":{"type":"http","url":"http://127.0.0.1:48731/mcp"},"transport":"http","scope":"dynamic","source":"dynamic"}]}}}
```

What the CLI asked the server for until the first turn was done (driver note; no request had an `Authorization` header):

```text
POST  /mcp                                          server/discover -> 401
GET   /.well-known/oauth-protected-resource/mcp      -> 404
GET   /.well-known/oauth-protected-resource          -> 404
GET   /.well-known/oauth-authorization-server        -> 404
GET   /.well-known/openid-configuration              -> 404
POST  /register                                      -> 401
POST  /mcp                                          initialize -> 401
GET   /.well-known/oauth-authorization-server        -> 404
GET   /.well-known/openid-configuration              -> 404
GET   /.well-known/oauth-protected-resource/mcp      -> 404
GET   /.well-known/oauth-protected-resource          -> 404
POST  /register                                      -> 401
```

What the adapter can rely on:
- A server that answers 401 everywhere is NOT shown as "needs sign-in": `mcp_status` says `"status":"pending"` while
  the CLI tries (asked right after the start), then `"status":"failed"` with
  `"error":"Dynamic Client Registration rejected (HTTP 401): {\"error\":\"unauthorized\"}"`. `system/init.mcp_servers`
  has `{"name":"needs-login","status":"failed","source":"dynamic"}` and no error text. The session runs.
- The headless CLI starts the OAuth steps by itself at connect: it looks up the `/.well-known/` documents and POSTs a
  client registration to `/register`, twice per connect (for `server/discover` and for `initialize`).
- `mcp_reconnect` `{"serverName"}` on it: a control ERROR whose `error` is the connect error text.
- `mcp_toggle` `{"serverName","enabled":false}`: success with no inner response; status becomes `disabled`.
  `mcp_toggle` with `enabled:true` tries to connect: here a control ERROR with the same connect error, and the status
  is `failed` again. So an error answer to "enable" means "enabled, but it did not connect".
- The status value `needs-auth` exists: scenario 19's init line had it for servers of the user's own setup (removed
  from the fixture). A recording of it for a server of the scenario is NOT verified: it needs a fake server with working
  OAuth documents and client registration, and the CLI may then save that registration in the login keychain.

### 19. A `.mcp.json` in the work dir, `--setting-sources project`, no `--strict-mcp-config`

Question: does the headless CLI start the project's MCP server without any approval? What does init show?

Fixtures: `r4-project-mcp-project` (`--permission-mode manual --setting-sources project --tools Bash`), `r4-project-mcp-strict` (`--permission-mode manual --strict-mcp-config --setting-sources project --tools Bash`). `.mcp.json` in the work dir:

```json
{"mcpServers":{"ping-project":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]}}}
```

`mcp_ping_server.py` (in the drivers folder) is a minimal stdio MCP server with one tool, `ping`; it adds a line to `started.txt` when it starts.

Without `--strict-mcp-config` (driver notes, init line, the tool request and result):

```json
# t=3.01
{"_meta":{"before_first_message":true,"server_started":true},"_t":3.012}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash","mcp__ping-project__ping"],"mcp_servers":[{"name":"ping-project","status":"connected","source":"project"}],"…":"…"}
# t=3.71
{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"mcp__ping-project__ping","mcp_server":{"name":"ping-project","source":"project"},"display_name":"Ping","input":{},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"mcp__ping-project__ping"}],"behavior":"allow","destination":"localSettings"}],"server_policy_ask_only":true,"tool_use_id":"toolu-1"}}
# t=3.95
{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-1","type":"tool_result","content":[{"type":"text","text":"pong"}]}]},"tool_use_result":[{"type":"text","text":"pong"}],"tool_result_meta":[{"id":"toolu-1","permission_decision":{"decision":"accept","source":"user_temporary"}}],"…":"…"}
# t=4.87
{"_meta":{"server_started":true,"marker":1,"init_entry_for_this_server":[{"name":"ping-project","status":"connected","source":"project"}],"can_use_tool_requests":[["mcp__ping-project__ping",null]],"servers_of_the_users_own_setup_in_init":true},"_t":4.868}
```

With `--strict-mcp-config` (same folder contents):

```json
# t=3.01
{"_meta":{"before_first_message":true,"server_started":false},"_t":3.013}
# t=3.01
{"type":"system","subtype":"init","model":"claude-haiku-5-5","permissionMode":"default","tools":["Bash"],"mcp_servers":[],"…":"…"}
# t=4.04
{"_meta":{"server_started":false,"marker":0,"init_entry_for_this_server":[],"can_use_tool_requests":[],"servers_of_the_users_own_setup_in_init":false},"_t":4.044}
```

What the adapter can rely on:
- Yes. In `-p` mode the server of the project's `.mcp.json` was started with no approval step, within 3 s of the
  spawn and before the first user message (`server_started:true`). The init line has
  `{"name":"ping-project","status":"connected","source":"project"}` and the tool `mcp__ping-project__ping` in `tools`.
- Calling the tool asks, in mode `manual`: `can_use_tool` with `"mcp_server":{"name","source":"project"}`,
  `"display_name":"Ping"`, `"server_policy_ask_only":true` and an `addRules` suggestion for the tool name.
- With `--strict-mcp-config` the same `.mcp.json` is ignored: the server was not started, `mcp_servers` is `[]`.
- Without `--strict-mcp-config`, `--setting-sources project` does NOT keep the user's own setup out: the init line
  also had the user's claude.ai connectors (`"source":"claudeai"`, states `connected` and `needs-auth`), plugins and
  skills. They are removed from the fixture (`servers_of_the_users_own_setup_in_init:true` is all that is left).

## F. Images

### 20. A PNG of 1 to 3 MB as an image block

Fixture: `r4-big-image`. Flags: `--permission-mode acceptEdits --strict-mcp-config --setting-sources local --tools ''`.

The PNGs are made in the driver with `zlib` + `struct`: RGB noise from `random.Random(20).randbytes` (so it does not compress), 800x800 and 1000x1000, never written to disk. The fixture has `<base64 removed: N characters>` in place of the data.

```json
# t=0.05
{"_meta":{"image":{"width":800,"height":800,"png_bytes":1921448,"base64_chars":2561932}},"_t":0.054}
# t=0.05
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"<base64 removed: 2561932 characters>"}},{"type":"text","text":"Is this picture a photo or random noise? One word."}]},"parent_tool_use_id":null},"_t":0.054}
# t=1.12
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"Noise"}],"…":"…"},"…":"…"}
# t=1.12
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"Noise","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":0,"…":"…"}
# t=1.42
{"_meta":{"image":{"width":1000,"height":1000,"png_bytes":3001978,"base64_chars":4002640}},"_t":1.418}
# t=1.42
{"_sent":{"type":"user","session_id":"","message":{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"<base64 removed: 4002640 characters>"}},{"type":"text","text":"Is this picture a photo or random noise? One word."}]},"parent_tool_use_id":null},"_t":1.418}
# t=2.12
{"type":"assistant","message":{"model":"claude-haiku-5-5","content":[{"type":"text","text":"Noise"}],"…":"…"},"…":"…"}
# t=2.12
{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"Noise","stop_reason":"end_turn","terminal_reason":"completed","queued_turn_count":0,"result_index":1,"…":"…"}
```

`result.usage` of the two turns: `input_tokens` 2, `cache_creation_input_tokens` 4042, `cache_read_input_tokens` 649; `input_tokens` 2, `cache_creation_input_tokens` 1889, `cache_read_input_tokens` 4289.

What the adapter can rely on:
- A 1.92 MB PNG (2.56 M base64 characters) and a 3.00 MB PNG (4.00 M characters), each as ONE stdin line of
  stream-json, were accepted: a normal turn, a sensible answer (`Noise`), no error or warning line, stderr empty.
  The usage numbers fit an image that reached the model: the second turn made 1,889 new cache tokens for a message
  whose text is ten words (inferred from usage; the answer alone could be a guess).
- Not verified: where the size limit is (nothing above 3 MB was sent), and what the error looks like there.

## What this means for `crates/codly-core/src/adapters/claude.rs` (as of the day of recording)

Confirmed:
- `on_interrupt_answer`: `still_queued` holds plain uuid strings, so "its uuid is anywhere in the list" is right; a
  message the list names runs as the next turn by itself and must not be sent again (scenario 1). The comment there
  ("what an entry looks like is not recorded") can now name the shape.
- `on_lifecycle`: `started` while no turn is open = the message's own turn; `started` while a turn runs = folded in.
  Two waiting messages that start together after a turn give two `started` lines and one turn (scenario 2), which
  the current code reports as one turn start plus one steer: the same thing in the end.
- Holding a slash command until the turn ends matches what the CLI does itself (scenario 3).
- `SANDBOX_NETWORK` (`SandboxNetworkAccess`, `input.host`) is now recorded, not only read in the bundle (scenario 14).

Differs from a comment or an assumption in the code:
- "killpg() then also reaches the shells and test runners the agent started" (`proc.rs`, `command` and `reap`): not
  for this CLI. Every Bash call's shell is its own process group and session; the CLI's group is the CLI alone
  (scenario 7). SIGTERM to the CLI makes the CLI end its tracked background tasks; SIGKILL leaves them running; a
  child started with `&` survives everything, the sweep of the group after the exit included.
- "Claude has no live command output" (`tool_progress` arm): `get_task_output` returns the tail of a running Bash
  command, foreground or background (scenarios 6, 8). `tool_progress` itself is only a 30 s heartbeat whose
  `tool_use_id` is `<call id>-heartbeat-N`.
- "Workers edit freely inside their own worktree (Bash still asks …)" (`permission_mode`): in `acceptEdits`, Bash
  `mv` / `rm` / `mkdir` inside the work dir do not ask (scenario 9).
- `close_stdin` ("so that the CLI exits by itself"): with an open background task the CLI does not exit; it waits for
  the task and then runs a turn (scenario 7).
- The shutdown path ends the process when a prompt still waits in the CLI. `interrupt` with `cancel_queued:true`
  makes the CLI drop those prompts itself and says which (scenario 1).
- A `cancelled` lifecycle line is treated as "the CLI dropped it: send it again". It is also what the CLI writes for a
  prompt the HOST withdrew (`cancel_async_message`, `cancel_queued`), before the answer (scenarios 1, 5). Not a bug
  today (the adapter sends neither), but it will be the day it does.
- `system/vcs_state_changed` (after a commit) is not in the list of known system subtypes, so it becomes an
  `Unknown` row (scenario 17).
- "did not enable requested tools": with the app's own tool list for a 1:1 agent, `WebFetch` was not in `init.tools`
  in this recording (`--setting-sources local`), and it is not in `OPTIONAL_TOOLS`, so this run would show that
  message. Whether the app's real flags (`--setting-sources user…`) give the same is not recorded (scenario 10).
- The model shown for a turn: in plan mode a `haiku` session runs on sonnet while `init.model` says haiku (scenario 10).
- `settings_json` passes `failIfUnavailable:true`; what it does when the sandbox cannot start is still not recorded
  (scenario 16).
- An MCP server that needs a sign-in can show as `failed` (scenario 18), so `on_init` asks `mcp_status` for it and
  reports it as a server that "did not start", with the OAuth error as the reason; only `needs-auth` is left alone.
- A process that exits with code 1 after stdin was closed is not a crash by itself: seen when the last turn was the
  interrupted one (scenario 8, n=1).

## Table: question -> answer -> fixture

| # | question | answer | fixture |
|---|---|---|---|
| 1 | Interrupt while a message waits | `still_queued` = `[<uuid>]`; the turn's own message gets `cancelled`; the waiting one runs as its own turn at once. With `cancel_queued:true`: `cancelled` line + `{"still_queued":[],"cancelled":[<uuid>]}`, message dropped | `r4-queue-interrupt-plain`, `r4-queue-interrupt-cancel-queued` |
| 2 | Two messages mid-turn | Both `queued`, both `started` at the next tool boundary in order, ONE result, `user_message_uuids` = all. No boundary: both start together after the result as ONE new turn | `r4-two-queued-tools`, `r4-two-queued-notool` |
| 3 | Slash command mid-turn | Accepted, not folded, runs as its own turn right after the running one | `r4-slash-mid-turn-context`, `r4-slash-mid-turn-compact` |
| 4 | Background task ends while a message waits | The running turn takes the message at the next tool boundary (and the notification); `priority:"later"` = own turn after. No self-started turn | `r4-background-and-queue-fold`, `r4-background-and-queue-later` |
| 5 | `cancel_async_message` | `{"message_uuid"}` -> `cancelled` lifecycle line, then `{"cancelled":true}`; `false` when taken, repeated, or the turn's own | `r4-cancel-async-message` |
| 6 | Life of a background task; `stop_task` | `task_started` + `background_tasks_changed` (the list); end: `task_updated`, `task_notification`, list `[]`, self-started turn. No list request. `stop_task`: `killed`/`stopped` lines, then `{}`, no turn | `r4-background-life` |
| 7 | Process groups; what outlives the CLI | Each Bash shell = own group + session, child of the CLI. `&` child survives all. Background task: gone after SIGTERM, alive after SIGKILL; stdin close makes the CLI wait for it. A command starting with `sleep N` (N >= 25) is refused | `r4-process-groups-blocked`, `-stdin`, `-sigterm`, `-sigkill` |
| 8 | Lines during a long tool; interrupt mid-tool | `task_started` after 3-4 s; `tool_progress` heartbeat every 30 s from 30 s on; nothing else. Interrupt -> result within 0.01 s (CLI time stamps) | `r4-tool-progress`, `r4-tool-progress-output` |
| 9 | `acceptEdits`: `mv`/`rm`; edit of local settings | `mv`, `rm`, `mkdir` in the work dir run without a request; Edit of `.claude/settings.local.json` asks (`safetyCheck`) | `r4-accept-edits` |
| 10 | Plan mode from default; unknown tool names | `ExitPlanMode` usable after `set_permission_mode` plan. Unknown `--tools` names: dropped silently, only `init.tools` shows it. Plan mode ran on sonnet | `r4-plan-from-default`, `r4-unknown-tool-names` |
| 11 | `ExitPlanMode`: changed plan; deny + message | Changed `updatedInput.plan` becomes the plan (tool result "edited by user", plan file rewritten, model follows it). Deny: error tool result, model revises | `r4-plan-from-default` |
| 12 | `/compact` with almost no history | Always a `result`. Empty: synthetic `Error: No messages to compact` + success result. One turn: full compaction | `r4-compact-empty` |
| 13 | `set_model` default; effort; `--fallback-model` | `default` -> `claude-opus-5-5`. `apply_flag_settings {effortLevel}` -> success, visible in `get_settings`; a bad level also answers success. Fallback model shows nowhere | `r4-model-effort-fallback` |
| 14 | Sandbox + `curl` | `can_use_tool` `SandboxNetworkAccess` `{host}` with a `WebFetch` `domain:` suggestion and its own uuid `tool_use_id`. Allow: normal result via proxy. Deny: exit 56, `<sandbox_violations>` | `r4-sandbox-network-allow`, `r4-sandbox-network-deny` |
| 15 | `autoAllowBashIfSandboxed:false` | Bash is back under the permission mode: asks in `manual`, not in `acceptEdits`. With auto-allow, `manual` does not ask either | `r4-sandbox-noauto`, `r4-sandbox-manual`, `r4-sandbox-manual-noauto` |
| 16 | `failIfUnavailable`; sandbox status | Status: `get_sandbox_dialog`. Failure text: not verified | `r4-sandbox-required-available`, `r4-sandbox-required-unavailable` |
| 17 | Sandbox + linked worktree + `git commit` | Not verified (work folder lies in a tree the sandbox lets commands write). Recorded: the commit command asks even with the sandbox on; `vcs_state_changed` line | `r4-sandbox-worktree-linked`, `-plain`, `-linked-off` |
| 18 | MCP server that needs sign-in | `pending`, then `failed` + "Dynamic Client Registration rejected (HTTP 401)"; `mcp_reconnect` and `mcp_toggle` on = control error with that text; toggle off = `disabled` | `r4-mcp-needs-auth` |
| 19 | Project `.mcp.json`, headless | Started with no approval before the first message (`connected`, `source:"project"`); ignored with `--strict-mcp-config` | `r4-project-mcp-project`, `r4-project-mcp-strict` |
| 20 | PNG of 1-3 MB | 1.92 MB and 3.00 MB accepted, no error text | `r4-big-image` |

## Not verified

- 1: an interrupt while SEVERAL messages wait (order in `still_queued`); a message already folded into the turn when
  the interrupt comes (does it get `cancelled`, is it put back: the bundle has `put_back` and
  `requeue_if_interrupted_within_ms`); `interrupt` with the bundle's `send_now` member. One message, one run each.
- 2: more than two messages; a mix of priorities.
- 4: the same with a background SUBAGENT; a task that fails.
- 6: `background_tasks` on a running foreground task (it answered `{}` with none running); `get_task_output` after the
  output file is gone; `truncated:true` (output over 8 KiB, bundle).
- 7: Linux; the exact `sleep` limit (25 is from the bundle; 21 ran, 301 was refused); children of a subagent.
- 8: `tool_progress` for tools other than Bash; the heartbeat past 90 s.
- 9: `git commit`, `chmod`, writes outside the work dir in `acceptEdits` (part 1 has one outside write: it asks).
- 10: why `WebFetch` is missing from `init.tools`, and whether it is with the app's own `--setting-sources`.
- 13: a turn after `set_model` "default" (it would run on opus; the rule of this recording was haiku only); a real
  fallback to the fallback model (`model_fallback` lines exist in the bundle).
- 14: a second request to the same host in the session (bundle: the host stays allowed); `updatedPermissions` on the
  allow; a host denied by settings.
- 16: the text printed when `failIfUnavailable` stops the start (needs a machine where the sandbox cannot start, or
  managed settings).
- 17: the question itself (see the section): needs a work folder outside the CLI's temp tree.
- 18: the `needs-auth` state for a scenario server; `mcp_authenticate`, `mcp_clear_auth`, `mcp_oauth_callback_url`
  (they start a real sign-in and may write to the login keychain).
- 19: a project server with `--setting-sources user,project` (the app's flags with the user's setup), and whether
  `enableAllProjectMcpServers` / `disabledMcpjsonServers` in settings change it.
- 20: images above 3 MB, other formats, several images in one message.
