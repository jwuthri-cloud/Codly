"""Builds the part-2 document from the sanitized r4-* fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes_part2.py [output file]      (default: $CAPTURE_DIR/NOTES-part2.md)
"""
import json
import os
import sys

from notes_lib import OUT, argv_flags, block, control, first, fx, j, quiet, sent, slim, timeline

P = []
w = P.append


def flags(name):
    return "`%s`" % argv_flags(name)


def typ(*types):
    return lambda l: quiet(l) and (l.get("type") in types or ("_sent" in l and "sent" in types) or ("_meta" in l and "meta" in types))


def life(l):
    """The lines that tell the story of a queue: what was sent, lifecycle, results, interrupt answers."""
    if not quiet(l):
        return False
    if "_sent" in l or "_meta" in l:
        return "closing_stdin" not in json.dumps(l) and "exit_code" not in json.dumps(l)
    return l.get("type") in ("command_lifecycle", "result", "control_response") or (l.get("type") == "system" and l.get("subtype") in ("init",))


def life_tools(l):
    return life(l) or (l.get("type") == "user" and is_tool_result(l)) or (l.get("type") == "assistant" and any(b["type"] == "text" for b in l["message"]["content"]))


def is_tool_result(l):
    c = l.get("message", {}).get("content")
    return isinstance(c, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in c)


def tasks(l):
    return quiet(l) and (l.get("type") == "system" and l.get("subtype") in ("task_started", "task_updated", "task_notification", "background_tasks_changed"))


def req(name, tool=None, n=None):
    """can_use_tool requests of a fixture, each followed by the driver's answer."""
    lines = fx(name)
    out = []
    for i, l in enumerate(lines):
        if l.get("type") == "control_request" and l["request"].get("subtype") == "can_use_tool" and (tool is None or l["request"]["tool_name"] == tool):
            out.append(j(l))
            for m in lines[i + 1:]:
                if "_sent" in m and m["_sent"].get("type") == "control_response" and m["_sent"]["response"].get("request_id") == l["request_id"]:
                    out.append(j(m["_sent"]))
                    break
    return out if n is None else out[:2 * n]


def metas(name, key=None):
    return [j(l) for l in fx(name) if "_meta" in l and (key is None or key in l["_meta"])]


w("""# Claude Code CLI stream-json: recorded protocol shapes, part 2

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
""")

# ================================================================= A
w("## A. Queue and stop\n")

# ---- 1
w("### 1. A message waits in the queue, then the host sends `interrupt`\n")
w("Question: what does the interrupt response hold, which `command_lifecycle` lines come, and does the waiting message run or is it dropped?\n")
w("Fixtures: `r4-queue-interrupt-plain`, `r4-queue-interrupt-cancel-queued`. Flags: %s.\n" % flags("r4-queue-interrupt-plain"))
w("Sent: a turn with one `sleep 15`; 1 s after the tool started, a second user message; 1 s later the interrupt.\n")
w(block([j(x) for x in sent("r4-queue-interrupt-plain")[:3]]))
w("Plain `interrupt` (no fields): every line from the second message on, without the tool result lines:\n")
w(block(timeline("r4-queue-interrupt-plain", life, start=lambda l: "_sent" in l and "banana" in json.dumps(l), stop=lambda l: "_meta" in l and "results_seen" in l["_meta"])))
w("`interrupt` with `\"cancel_queued\":true`:\n")
w(block(timeline("r4-queue-interrupt-cancel-queued", life, start=lambda l: "_sent" in l and "banana" in json.dumps(l), stop=lambda l: "_meta" in l and "results_seen" in l["_meta"])))
w("""What the adapter can rely on:
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
""")

# ---- 2
w("### 2. Two user messages written mid-turn\n")
w("Question: order of `command_lifecycle`, one result or several, `result.user_message_uuids`.\n")
w("Fixtures: `r4-two-queued-tools` (the turn has tool boundaries: three `sleep 4`), `r4-two-queued-notool` (a text-only turn). Flags: %s.\n" % flags("r4-two-queued-tools"))
w("With tool boundaries (lifecycle lines, tool results, final text, result):\n")
w(block(timeline("r4-two-queued-tools", brief=True, pred=life_tools)))
w("Text-only turn (no boundary to fold at): both messages wait for the turn's end:\n")
w(block(timeline("r4-two-queued-notool", lambda l: life(l) or (l.get("type") == "system" and l.get("subtype") == "status"), text=40)))
w("""What the adapter can rely on:
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
""")

# ---- 3
w("### 3. A slash command written mid-turn\n")
w("Question: is it folded into the turn, run after it, or refused?\n")
w("Fixtures: `r4-slash-mid-turn-context` (`/context`), `r4-slash-mid-turn-compact` (`/compact`). Flags: %s. The turn runs `sleep 4` twice; the command is written during the first.\n" % flags("r4-slash-mid-turn-context"))
w(block(timeline("r4-slash-mid-turn-context", brief=True, pred=lambda l: life(l) or (l.get("type") == "user" and is_tool_result(l)), text=60)))
w("`/compact` (from the first turn's result on):\n")
w(block(timeline("r4-slash-mid-turn-compact", lambda l: life(l) or (l.get("type") == "system" and l.get("subtype") in ("status", "compact_boundary")),
                 start=lambda l: l.get("type") == "result")))
w("""What the adapter can rely on:
- A slash command written mid-turn is accepted (`queued`), is NOT folded at a tool boundary (two boundaries passed),
  and runs as a turn of its own right after the running turn's `result`: `started`, the command's normal lines
  (part 1, section 11), its own `result` with `local_command`, `completed`.
- So the CLI already does what the adapter does by holding a slash command back until the turn ends.
""")

# ---- 4
w("### 4. A background Bash task ends while a user message waits\n")
w("Question: which turn takes the message?\n")
w("Fixtures: `r4-background-and-queue-fold` (message with no priority), `r4-background-and-queue-later` (`priority:\"later\"`). Flags: %s.\n" % flags("r4-background-and-queue-fold"))
w("Turn 1 starts `sleep 12; echo woke` in the background. Turn 2 runs a foreground `sleep 14`; 1 s into it the third message is written; the background task ends 9.6 s later, while the message waits.\n")
st4 = lambda l: "_sent" in l and "sleep 14" in json.dumps(l)
w("No priority (from the start of turn 2):\n")
w(block(timeline("r4-background-and-queue-fold", brief=True, pred=lambda l: life_tools(l) or tasks(l), start=st4)))
w("`priority:\"later\"` (from the start of turn 2):\n")
w(block(timeline("r4-background-and-queue-later", brief=True, pred=lambda l: life_tools(l) or tasks(l), start=st4)))
w("""What the adapter can rely on:
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
""")

# ---- 5
w("### 5. `cancel_async_message` on a really queued message\n")
w("Question: request and response shape, and what else is written.\n")
w("Fixture: `r4-cancel-async-message`. Flags: %s. The turn runs `sleep 6` twice; two messages are written during the first.\n" % flags("r4-cancel-async-message"))
w(block(timeline("r4-cancel-async-message", brief=True, pred=life_tools, start=lambda l: "_sent" in l and "banana" in json.dumps(l))))
w("""What the adapter can rely on:
- Request: `{"subtype":"cancel_async_message","message_uuid":<uuid>}`. Answer: `{"cancelled":true|false}`.
- On a message that still waits: `command_lifecycle` `cancelled` for it is written BEFORE the response
  `{"cancelled":true}`. The message never reaches the model (the answer was `done cherry`, no `banana`) and is not in
  `result.user_message_uuids`.
- `{"cancelled":false}`, with no other line: for the same uuid again, for a message a turn already took (`started`
  was written), and for the message that opened the running turn.
- A host that withdraws a message this way must forget the uuid before it sends the request: the `cancelled` line
  comes first, and it looks exactly like the `cancelled` of a message the CLI gave up by itself.
""")

# ================================================================= B
w("## B. Background work and the end of a turn\n")

# ---- 6
w("### 6. The full life of a background task, `get_task_output`, `stop_task`\n")
w("Question: which lines tell the host that a background task is open, when it started and when it ended? Is there a control request that lists open tasks? What does `stop_task` do on a real task id?\n")
w("Fixture: `r4-background-life`. Flags: %s.\n" % flags("r4-background-life"))
w("Task 1 (`sleep 8; echo woke`, `run_in_background:true`) ends by itself. Every task line, control request and result up to the start of task 2:\n")
bl = lambda l: tasks(l) or (quiet(l) and (l.get("type") in ("result", "control_response") or ("_sent" in l and l["_sent"].get("type") == "control_request") or (l.get("type") == "system" and l.get("subtype") == "init") or (l.get("type") == "user" and is_tool_result(l))))
w(block(timeline("r4-background-life", bl, stop=lambda l: l.get("type") == "control_response" and l["response"].get("request_id") == "host-4")))
w("Task 2 (`sleep 127; echo late`) is stopped by the host 2 s after the turn ended:\n")
w(block(timeline("r4-background-life", lambda l: bl(l) or "_meta" in l, start=lambda l: "_sent" in l and "sleep 127" in json.dumps(l))))
w("""What the adapter can rely on:
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
  wrote. Running and silent: `""`. Ended: `"woke\\n\\n[exited with code 0]\\n"`. Stopped: `"\\n[killed]\\n"`.
  It works for a foreground Bash call too, once its `task_started` line gave the id (scenario 8).
- `stop_task` `{"task_id"}` on a running task: `task_updated` (`patch.status:"killed"`), `task_notification` with
  `status:"stopped"`, `background_tasks_changed` `[]`, THEN the response `{}`. The command's process was gone when
  the host looked 15 s later. NO turn starts by itself after a stop (none in those 15 s).
- `stop_task` answers `{}` (success) also for a task that already ended, for one stopped before, and for an unknown
  id (part 1): the answer does not say whether anything was stopped; the task lines do.
""")

# ---- 7
w("### 7. Process groups of Bash children, and what outlives the CLI\n")
w("Question: which process group do Bash children get, and are they alive after the CLI exits (stdin closed) and after SIGTERM to the CLI?\n")
w("Fixtures: `r4-process-groups-blocked`, `r4-process-groups-stdin`, `r4-process-groups-sigterm`, `r4-process-groups-sigkill`. Flags: %s. The driver starts the CLI as the leader of its own process group (`process_group=0`, as the app does).\n" % flags("r4-process-groups-stdin"))
w("First finding: the two commands as first planned never ran. The CLI refuses a Bash command whose first part is a long `sleep` (`r4-process-groups-blocked`):\n")
w(block(timeline("r4-process-groups-blocked", lambda l: quiet(l) and (("_sent" in l) or (l.get("type") == "user" and is_tool_result(l)) or (l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"]))), text=60)))
w("The bundle's rule (bundle, not recorded as a number): the first sub-command matches `^sleep\\s+(\\d+(?:\\.\\d*)?)\\s*$` and the number is 25 or more. `sleep 20` and `sleep 21` ran (scenario 8). So the runs below use `echo start; sleep 301 &`, plus a `sleep 47` with `run_in_background:true`.\n")
w("The `ps` the model ran in a foreground Bash call (`r4-process-groups-stdin`; `50599` is the `sleep 301`, found by the host):\n")
ps_line = first("r4-process-groups-stdin", lambda l: l.get("type") == "user" and is_tool_result(l) and "PGID" in json.dumps(l))
w(block([j({"type": "user", "message": ps_line["message"], "…": "…"}, maxstr=1200)]))
w("The host's own `ps` of the same processes while the CLI is alive, then what is left after the CLI ended, per variant (driver notes):\n")
for v in ("stdin", "sigterm", "sigkill"):
    w("`r4-process-groups-%s`:\n" % v)
    w(block([j(l, maxstr=600, keep=("host_ps", "children_of_cli", "group_of_cli")) for l in fx("r4-process-groups-" + v)
             if "_meta" in l and any(k in l["_meta"] for k in ("when", "sending", "closing_stdin", "exit_code", "cli_pid")) and "s_after" not in l["_meta"]]))
w("After stdin was closed, with the `sleep 47` task still open (`r4-process-groups-stdin`): the lines the CLI wrote before it exited:\n")
w(block(timeline("r4-process-groups-stdin", lambda l: quiet(l) and "_sent" not in l and (l.get("type") == "result" or tasks(l) or (l.get("type") == "system" and l.get("subtype") == "init")
                                                                      or ("_meta" in l and ("closing_stdin" in l["_meta"] or "exit_code" in l["_meta"]))),
                 start=lambda l: "_meta" in l and "closing_stdin" in l["_meta"], maxstr=200)))
w("""What the adapter can rely on:
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
""")

# ---- 8
w("### 8. Lines during a long tool call, and an interrupt mid-tool\n")
w("Question: which lines arrive while a tool runs, how often? How long from the `interrupt` request to the `result`?\n")
w("Fixtures: `r4-tool-progress` (`sleep 20`, then `sleep 21` interrupted after 8 s), `r4-tool-progress-output` (a 96 s loop that prints a line every 4 s). Flags: %s.\n" % flags("r4-tool-progress"))
w("`sleep 20`: every line from the `tool_use` line to the tool result, stream events included (the three that close the model's message, then only the two task lines):\n")
w(block(timeline("r4-tool-progress", lambda l: "_note" not in l and set(l) != {"_t"} and l.get("type") != "rate_limit_event",
                 start=lambda l: l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"]),
                 stop=lambda l: l.get("type") == "user" and is_tool_result(l))))
w("The 96 s command: task lines, `tool_progress` lines, the `get_task_output` request sent 6 s after `task_started`, the result:\n")
w(block(timeline("r4-tool-progress-output", lambda l: tasks(l) or (quiet(l) and (l.get("type") in ("tool_progress", "result", "control_response") or ("_sent" in l and l["_sent"].get("type") == "control_request"))))))
w("Interrupt 8 s into `sleep 21` (from the request to the driver's notes):\n")
w(block(timeline("r4-tool-progress", lambda l: quiet(l) and l.get("type") != "assistant", start=lambda l: "_meta" in l and "sleep_21_alive_before_interrupt" in l["_meta"],
                 stop=lambda l: "_meta" in l and "exit_code" in l["_meta"])))
w("""What the adapter can rely on:
- While a Bash call runs the CLI writes: `system/task_started` (`is_backgrounded:false`) 3 to 4 s after the
  `tool_use` line (a call that ends sooner gets none: over all part 2 recordings, none of the 39 foreground Bash
  calls that took up to 3.6 s has one, each of the 14 that took 4 s or more has one), then nothing until the first
  `tool_progress` at 30 s:
  `{"type":"tool_progress","tool_use_id":"<call id>-heartbeat-N","tool_name":"Bash","parent_tool_use_id":"<call id>",
  "elapsed_time_seconds":30|60|90,"heartbeat":true}`, one every 30 s. The call's own id is in `parent_tool_use_id`,
  not in `tool_use_id`. A 20 s call gets no `tool_progress` at all. The command's output is in none of these lines.
- Live output is available on request: `get_task_output` with the `task_id` of `task_started` answered the tail of
  the running foreground command (`"tick 1\\ntick 2\\ntick 3\\n"`).
- Interrupt mid-tool: the response came after 0.003 s; `task_notification` with `status:"stopped"`, the rejected
  `tool_result`, the `[Request interrupted by user for tool use]` line and the error `result` all carry the same time
  stamp (within 0.01 s of the request; the driver's own polling saw the result after 0.2 s). The `sleep 21` process was
  gone 1 s later.
- Exit code after stdin close was 1 in this run, where the last `result` was the interrupted (error) one; it was 0
  in every run whose last `result` was a success (n=1 for the error case).
""")

# ================================================================= C
w("## C. Modes, model, plan\n")

# ---- 9
w("### 9. `--permission-mode acceptEdits`: `mv`, `rm`, and an edit of `.claude/settings.local.json`\n")
w("Question: do `rm` / `mv` in Bash ask (`can_use_tool`) or run? Does an Edit of `.claude/settings.local.json` inside the work dir ask?\n")
w("Fixture: `r4-accept-edits`. Flags: %s. The work dir has a `.claude/settings.local.json` holding `{}` before the start.\n" % flags("r4-accept-edits"))
w(block([j(x) for x in sent("r4-accept-edits", "user")]))
w("Tool calls, results and requests (the driver's note after each turn counts the `can_use_tool` requests of that turn):\n")
w(block(timeline("r4-accept-edits", lambda l: quiet(l) and ((l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"])) or (l.get("type") == "user" and is_tool_result(l)) or l.get("type") == "control_request" or ("_meta" in l and ("turn" in l["_meta"] or "files" in l["_meta"]))), maxstr=260)))
w("""What the adapter can rely on:
- In `acceptEdits` the Write tool and the Bash commands `mv`, `rm`, `rm -rf sub && mkdir sub && mv … && mv …` on
  paths inside the work dir ran with NO `can_use_tool` request (`permission_decision` `source:"config"`, `reason_type`
  `mode` for the file tools and `subcommandResults` for Bash). `rm` of a file that git tracks included.
- An Edit of `.claude/settings.local.json` DOES ask, in the same mode: `can_use_tool` with
  `"decision_reason_type":"safetyCheck"`, a `decision_reason` text, `"classifier_approvable":true` and an empty
  `permission_suggestions`. After the allow the file was changed.
""")

# ---- 10
w("### 10. Plan mode set on a session started as the app starts it; tool names that do not exist\n")
w("Question: is `ExitPlanMode` usable after `set_permission_mode` plan on a session started in the default mode with the app's tool list? What does the CLI say about a tool name that does not exist?\n")
w("Fixtures: `r4-plan-from-default`, `r4-unknown-tool-names`.\n")
w("`r4-plan-from-default` flags: %s (the `--tools` list of `tools_for()` in `claude.rs` for a 1:1 agent with a shell).\n" % flags("r4-plan-from-default"))
ini = first("r4-plan-from-default", lambda l: l.get("type") == "control_response" and l["response"].get("request_id") == "host-1")
w("`initialize` says `\"current_permission_mode\":\"%s\"` for `--permission-mode manual`. Then:\n" % ini["response"]["response"]["current_permission_mode"])
w(block(control("r4-plan-from-default", "set_permission_mode") + [j(slim(first("r4-plan-from-default", lambda l: l.get("type") == "system" and l.get("subtype") == "init")), keep=("tools",))]))
w("`init.tools` has 16 names for the 18 asked: `Agent` is listed as `Task`; `WebFetch` and `LSP` are not there. No line says why.\n")
w("The models of the assistant lines in this one turn (same `--model haiku` process):\n")
models = []
for l in fx("r4-plan-from-default"):
    if l.get("type") == "assistant":
        b = l["message"]["content"][0]
        models.append("%s  %s" % (l["message"]["model"], b["type"] + (" " + b["name"] if b["type"] == "tool_use" else "")))
    if l.get("type") == "system" and l.get("subtype") == "status" and l.get("permissionMode"):
        models.append("-- system/status permissionMode=%s" % l["permissionMode"])
w(block(models, "text"))
w("`r4-unknown-tool-names` flags: %s. stderr was empty. The init line, and the rules the unknown `--allowedTools` names became:\n" % flags("r4-unknown-tool-names"))
rules = first("r4-unknown-tool-names", lambda l: l.get("type") == "control_response" and "rules" in json.dumps(l)[:300])
cli_rules = [r for r in rules["response"]["response"]["state"]["rules"] if r.get("source") == "cliArg"]
w(block([j(slim(first("r4-unknown-tool-names", lambda l: l.get("type") == "system" and l.get("subtype") == "init")), keep=("tools",))] + [j(r) for r in cli_rules]))
w("""What the adapter can rely on:
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
""")

# ---- 11
w("### 11. `ExitPlanMode` answered with deny + message, and with allow + a changed plan\n")
w("Question: what does the CLI do with a changed plan in `updatedInput`? And with deny + message?\n")
w("Fixture: `r4-plan-from-default` (same run as scenario 10).\n")
w(block([j(x) for x in sent("r4-plan-from-default", "user")]))
plan = lambda l: quiet(l) and ((l.get("type") == "control_request" and l["request"]["tool_name"] == "ExitPlanMode")
                               or ("_sent" in l and l["_sent"].get("type") == "control_response" and ("plan" in json.dumps(l["_sent"]) or "Not yet" in json.dumps(l["_sent"])))
                               or (l.get("type") == "system" and l.get("subtype") == "status" and l.get("permissionMode"))
                               or (l.get("type") == "user" and is_tool_result(l) and ("approved your plan" in json.dumps(l) or "Not yet" in json.dumps(l))))
w(block(timeline("r4-plan-from-default", plan, start=lambda l: l.get("type") == "control_request", maxstr=420)))
w("After the turn: `get_plan`, and the driver's look at the files:\n")
w(block(control("r4-plan-from-default", "get_plan") + metas("r4-plan-from-default", "hello_txt")))
w("""What the adapter can rely on:
- Both requests have `"requires_user_interaction":true` and no `permission_suggestions`.
- Deny + message: the `tool_result` is the message with `is_error:true`; the session stays in plan mode; the model
  changed its plan file and called `ExitPlanMode` again.
- Allow + changed `updatedInput.plan`: the CLI takes the changed text as the plan. `system/status` switches to
  `permissionMode:"default"`. The `tool_result` text says `## Approved Plan (edited by user):` followed by the changed
  text (an unchanged plan says `## Approved Plan:`, part 1). The plan FILE on disk is rewritten with the changed text
  (`get_plan.content`, and the file itself), and `tool_use_result` has `"planWasEdited":true`. The model then did
  what the changed plan says (`planet`, not `world`) and said so.
- After the approval the next Edit asked through `can_use_tool` (mode `default`).
""")

# ---- 12
w("### 12. `/compact` on a session with almost no history\n")
w("Question: is there a `result`? What lines come?\n")
w("Fixture: `r4-compact-empty`. Flags: %s. Sent: `/compact` as the very first message, one tiny turn, `/compact` again.\n" % flags("r4-compact-empty"))
w("`/compact` first (every line):\n")
w(block(timeline("r4-compact-empty", lambda l: quiet(l) and "_meta" not in l, stop=lambda l: l.get("type") == "result")))
w("`/compact` after one turn (every line but the summary text, which is cut):\n")
seen12 = {"n": 0}


def second_compact(l):
    if "_sent" in l and l["_sent"].get("message", {}).get("content") == "/compact":
        seen12["n"] += 1
    return seen12["n"] == 2


w(block(timeline("r4-compact-empty", lambda l: quiet(l) and "_meta" not in l, start=second_compact, stop=lambda l: l.get("type") == "result", maxstr=160)))
w("""What the adapter can rely on:
- Yes, always a `result`. With nothing to compact: `system/init`, one `assistant` line with `"model":"<synthetic>"`
  and the text `Error: No messages to compact`, then a `result` with `subtype:"success"`, `is_error:false`,
  `num_turns` 0, empty `result` text, `local_command:"compact"`, cost 0. No `status` `compacting`, no
  `compact_boundary`. The "error" is only in that text.
- After one tiny turn `/compact` runs in full (4.6 s): `status` `compacting`, `status` with
  `compact_result:"success"`, `system/init`, `compact_boundary` (`pre_tokens` 5765, `post_tokens` 934), the summary
  `user` line, the replayed `<local-command-stdout>Compacted </local-command-stdout>` line, the `result`.
""")

# ---- 13
w("### 13. `set_model` \"default\", `apply_flag_settings` with an effort level, `--fallback-model`\n")
w("Fixture: `r4-model-effort-fallback`. Flags: %s. No turn ran on another model than haiku.\n" % flags("r4-model-effort-fallback"))
ctl = control("r4-model-effort-fallback", maxstr=700)
keep13 = [x for x in ctl if '"initialize"' not in x and '"commands"' not in x and "get_status" not in x and '"sections"' not in x]
w(block(keep13))
w("""What the adapter can rely on:
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
""")

# ================================================================= D
w("## D. Sandbox\n")
w("Settings are passed as in part 1, section 9: `--settings '{\"sandbox\":{…}}'`.\n")

# ---- 14
w("### 14. Sandbox on + `curl -sI https://example.com`\n")
w("Question: the exact request the host gets, and what follows on allow and on deny.\n")
w("Fixtures: `r4-sandbox-network-allow`, `r4-sandbox-network-deny`. Flags: %s.\n" % flags("r4-sandbox-network-allow"))
net = lambda l: quiet(l) and ((l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"])) or l.get("type") == "control_request"
                              or ("_sent" in l and l["_sent"].get("type") == "control_response") or (l.get("type") == "user" and is_tool_result(l)))
w("Allow:\n")
w(block(timeline("r4-sandbox-network-allow", net, maxstr=520)))
w("Deny:\n")
w(block(timeline("r4-sandbox-network-deny", net, maxstr=520)))
w("""What the adapter can rely on:
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
""")

# ---- 15
w("### 15. `autoAllowBashIfSandboxed:false`, and the app's `manual` mode\n")
w("Question: do sandboxed Bash commands then ask through `can_use_tool`?\n")
w("Fixtures: `r4-sandbox-noauto` (%s), `r4-sandbox-manual` (%s), `r4-sandbox-manual-noauto` (%s).\n" % (flags("r4-sandbox-noauto"), flags("r4-sandbox-manual"), flags("r4-sandbox-manual-noauto")))
w("Three turns each: `echo hi > inside.txt`, `ls`, `mv inside.txt inside2.txt`. Requests per turn (driver notes) and the `mode` of `get_sandbox_dialog`:\n")
rows = ["| fixture | permission mode | `autoAllowBashIfSandboxed` | `get_sandbox_dialog.mode` | `echo hi > inside.txt` | `ls` | `mv` |", "|---|---|---|---|---|---|---|"]
for name, mode, auto in (("r4-sandbox-noauto", "acceptEdits", "false"), ("r4-sandbox-manual", "manual", "(default)"), ("r4-sandbox-manual-noauto", "manual", "false")):
    dlg = first(name, lambda l: l.get("type") == "control_response" and "supported" in json.dumps(l))["response"]["response"]["mode"]
    counts = [l["_meta"]["can_use_tool_requests"] for l in fx(name) if "_meta" in l and "turn" in l["_meta"]]
    rows.append("| `%s` | `%s` | %s | `%s` | %s | %s | %s |" % (name, mode, auto, dlg, *["asks" if c else "runs" for c in counts]))
w("\n".join(rows) + "\n")
w("The request for `echo hi > inside.txt` in `r4-sandbox-manual-noauto`:\n")
w(block(req("r4-sandbox-manual-noauto", "Bash", 1)))
w("""What the adapter can rely on:
- With the sandbox on and nothing else (auto-allow, `mode:"auto-allow"`), Bash commands do not ask, also when the
  session's permission mode is `manual`/`default`. Part 1 showed this for `acceptEdits` only.
- `autoAllowBashIfSandboxed:false` (`mode:"regular"`) puts Bash back under the permission mode: in `manual` the two
  writing commands asked with an ordinary `can_use_tool` (suggestions included, no sandbox marker in the request) and
  `ls` ran; in `acceptEdits` nothing asked, as without a sandbox (scenario 9). `get_sandbox_dialog` still says
  `enabled:true` in these runs.
""")

# ---- 16
w("### 16. `failIfUnavailable`, and how the host can see that the sandbox is on\n")
w("Fixtures: `r4-sandbox-required-available` (%s), `r4-sandbox-required-unavailable` (%s).\n" % (flags("r4-sandbox-required-available"), flags("r4-sandbox-required-unavailable")))
w("`get_sandbox_dialog` with `failIfUnavailable:true` on a machine where the sandbox works:\n")
w(block(control("r4-sandbox-required-available", "get_sandbox_dialog", 1, maxstr=900)))
w("""What the adapter can rely on:
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
""")

# ---- 17
w("### 17. Sandbox on, in a linked git worktree: can `git commit` write to the main repo's `.git`?\n")
w("Fixtures: `r4-sandbox-worktree-linked` (sandbox on, cwd = a `git worktree add` folder), `r4-sandbox-worktree-plain` (sandbox on, ordinary repo), `r4-sandbox-worktree-linked-off` (no sandbox). Flags: %s.\n" % flags("r4-sandbox-worktree-linked"))
w("Sent, and the driver's notes (`r4-sandbox-worktree-linked`):\n")
w(block([j(x) for x in sent("r4-sandbox-worktree-linked", "user")] + [j(l, maxstr=400) for l in fx("r4-sandbox-worktree-linked") if "_meta" in l and any(k in l["_meta"] for k in ("git_dir", "commits_after", "outside_probe_written"))]))
w("The request for the commit command (the same in all three fixtures), and the line the commit caused:\n")
w(block(req("r4-sandbox-worktree-linked", "Bash", 1) + [j(slim(first("r4-sandbox-worktree-linked", lambda l: l.get("type") == "system" and l.get("subtype") == "vcs_state_changed")))]))
w("""Answer: NOT verified. The commit was written (2 commits after, 1 before), but the recording cannot say that the
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
""")

# ================================================================= E
w("## E. MCP and project files\n")

# ---- 18
w("### 18. An MCP server that needs a sign-in\n")
w("Question: what do `system/init.mcp_servers` and `mcp_status` say? `mcp_reconnect` and `mcp_toggle` shapes.\n")
w("Fixture: `r4-mcp-needs-auth`. Flags: %s, with this file:\n" % flags("r4-mcp-needs-auth"))
w(block(['{"mcpServers":{"needs-login":{"type":"http","url":"http://127.0.0.1:48731/mcp"}}}']))
w("The server is a python3 `http.server` in the driver: every request gets `401` with `WWW-Authenticate: Bearer realm=\"needs-login\", error=\"invalid_token\", error_description=\"Sign in first\"`; paths under `/.well-known/` get `404`.\n")
w("Every control request with its answer, and the init line:\n")
i18 = first("r4-mcp-needs-auth", lambda l: l.get("type") == "system" and l.get("subtype") == "init")
c18 = control("r4-mcp-needs-auth", maxstr=600)
w(block(c18[:2] + [j(slim(i18))] + c18[2:]))
w("What the CLI asked the server for until the first turn was done (driver note; no request had an `Authorization` header):\n")
hits = first("r4-mcp-needs-auth", lambda l: "_meta" in l and "fake_server_requests" in l["_meta"])["_meta"]["fake_server_requests"]
w(block(["%-5s %-45s %s -> %d" % (h["method"], h["path"], h["jsonrpc_method"] or "", h["answer"]) for h in hits], "text"))
w("""What the adapter can rely on:
- A server that answers 401 everywhere is NOT shown as "needs sign-in": `mcp_status` says `"status":"pending"` while
  the CLI tries (asked right after the start), then `"status":"failed"` with
  `"error":"Dynamic Client Registration rejected (HTTP 401): {\\"error\\":\\"unauthorized\\"}"`. `system/init.mcp_servers`
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
""")

# ---- 19
w("### 19. A `.mcp.json` in the work dir, `--setting-sources project`, no `--strict-mcp-config`\n")
w("Question: does the headless CLI start the project's MCP server without any approval? What does init show?\n")
w("Fixtures: `r4-project-mcp-project` (%s), `r4-project-mcp-strict` (%s). `.mcp.json` in the work dir:\n" % (flags("r4-project-mcp-project"), flags("r4-project-mcp-strict")))
w(block(['{"mcpServers":{"ping-project":{"type":"stdio","command":"python3","args":["mcp_ping_server.py","started.txt"]}}}']))
w("`mcp_ping_server.py` (in the drivers folder) is a minimal stdio MCP server with one tool, `ping`; it adds a line to `started.txt` when it starts.\n")
w("Without `--strict-mcp-config` (driver notes, init line, the tool request and result):\n")
w(block(timeline("r4-project-mcp-project", lambda l: quiet(l) and (("_meta" in l and "closing_stdin" not in l["_meta"] and "exit_code" not in l["_meta"]) or (l.get("type") == "system" and l.get("subtype") == "init")
                                                                     or l.get("type") == "control_request" or (l.get("type") == "user" and is_tool_result(l))), maxstr=400)))
w("With `--strict-mcp-config` (same folder contents):\n")
w(block(timeline("r4-project-mcp-strict", lambda l: quiet(l) and (("_meta" in l and "server_started" in l["_meta"]) or (l.get("type") == "system" and l.get("subtype") == "init")))))
w("""What the adapter can rely on:
- Yes. In `-p` mode the server of the project's `.mcp.json` was started with no approval step, within 3 s of the
  spawn and before the first user message (`server_started:true`). The init line has
  `{"name":"ping-project","status":"connected","source":"project"}` and the tool `mcp__ping-project__ping` in `tools`.
- Calling the tool asks, in mode `manual`: `can_use_tool` with `"mcp_server":{"name","source":"project"}`,
  `"display_name":"Ping"`, `"server_policy_ask_only":true` and an `addRules` suggestion for the tool name.
- With `--strict-mcp-config` the same `.mcp.json` is ignored: the server was not started, `mcp_servers` is `[]`.
- Without `--strict-mcp-config`, `--setting-sources project` does NOT keep the user's own setup out: the init line
  also had the user's claude.ai connectors (`"source":"claudeai"`, states `connected` and `needs-auth`), plugins and
  skills. They are removed from the fixture (`servers_of_the_users_own_setup_in_init:true` is all that is left).
""")

# ================================================================= F
w("## F. Images\n")
w("### 20. A PNG of 1 to 3 MB as an image block\n")
w("Fixture: `r4-big-image`. Flags: %s.\n" % flags("r4-big-image"))
w("The PNGs are made in the driver with `zlib` + `struct`: RGB noise from `random.Random(20).randbytes` (so it does not compress), 800x800 and 1000x1000, never written to disk. The fixture has `<base64 removed: N characters>` in place of the data.\n")
w(block(timeline("r4-big-image", lambda l: quiet(l) and ("_meta" in l and "image" in l["_meta"] or "_sent" in l or l.get("type") in ("assistant", "result")))))
u20 = [l["usage"] for l in fx("r4-big-image") if l.get("type") == "result"]
w("`result.usage` of the two turns: " + "; ".join("`input_tokens` %d, `cache_creation_input_tokens` %d, `cache_read_input_tokens` %d" % (u["input_tokens"], u["cache_creation_input_tokens"], u["cache_read_input_tokens"]) for u in u20) + ".\n")
w("""What the adapter can rely on:
- A 1.92 MB PNG (2.56 M base64 characters) and a 3.00 MB PNG (4.00 M characters), each as ONE stdin line of
  stream-json, were accepted: a normal turn, a sensible answer (`Noise`), no error or warning line, stderr empty.
  The usage numbers fit an image that reached the model: the second turn made 1,889 new cache tokens for a message
  whose text is ten words (inferred from usage; the answer alone could be a guess).
- Not verified: where the size limit is (nothing above 3 MB was sent), and what the error looks like there.
""")

# ================================================================= adapter
w("""## What this means for `crates/codly-core/src/adapters/claude.rs` (as of the day of recording)

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
""")

# ================================================================= table
w("## Table: question -> answer -> fixture\n")
T = [
    ("1", "Interrupt while a message waits", "`still_queued` = `[<uuid>]`; the turn's own message gets `cancelled`; the waiting one runs as its own turn at once. With `cancel_queued:true`: `cancelled` line + `{\"still_queued\":[],\"cancelled\":[<uuid>]}`, message dropped", "`r4-queue-interrupt-plain`, `r4-queue-interrupt-cancel-queued`"),
    ("2", "Two messages mid-turn", "Both `queued`, both `started` at the next tool boundary in order, ONE result, `user_message_uuids` = all. No boundary: both start together after the result as ONE new turn", "`r4-two-queued-tools`, `r4-two-queued-notool`"),
    ("3", "Slash command mid-turn", "Accepted, not folded, runs as its own turn right after the running one", "`r4-slash-mid-turn-context`, `r4-slash-mid-turn-compact`"),
    ("4", "Background task ends while a message waits", "The running turn takes the message at the next tool boundary (and the notification); `priority:\"later\"` = own turn after. No self-started turn", "`r4-background-and-queue-fold`, `r4-background-and-queue-later`"),
    ("5", "`cancel_async_message`", "`{\"message_uuid\"}` -> `cancelled` lifecycle line, then `{\"cancelled\":true}`; `false` when taken, repeated, or the turn's own", "`r4-cancel-async-message`"),
    ("6", "Life of a background task; `stop_task`", "`task_started` + `background_tasks_changed` (the list); end: `task_updated`, `task_notification`, list `[]`, self-started turn. No list request. `stop_task`: `killed`/`stopped` lines, then `{}`, no turn", "`r4-background-life`"),
    ("7", "Process groups; what outlives the CLI", "Each Bash shell = own group + session, child of the CLI. `&` child survives all. Background task: gone after SIGTERM, alive after SIGKILL; stdin close makes the CLI wait for it. A command starting with `sleep N` (N >= 25) is refused", "`r4-process-groups-blocked`, `-stdin`, `-sigterm`, `-sigkill`"),
    ("8", "Lines during a long tool; interrupt mid-tool", "`task_started` after 3-4 s; `tool_progress` heartbeat every 30 s from 30 s on; nothing else. Interrupt -> result within 0.01 s (CLI time stamps)", "`r4-tool-progress`, `r4-tool-progress-output`"),
    ("9", "`acceptEdits`: `mv`/`rm`; edit of local settings", "`mv`, `rm`, `mkdir` in the work dir run without a request; Edit of `.claude/settings.local.json` asks (`safetyCheck`)", "`r4-accept-edits`"),
    ("10", "Plan mode from default; unknown tool names", "`ExitPlanMode` usable after `set_permission_mode` plan. Unknown `--tools` names: dropped silently, only `init.tools` shows it. Plan mode ran on sonnet", "`r4-plan-from-default`, `r4-unknown-tool-names`"),
    ("11", "`ExitPlanMode`: changed plan; deny + message", "Changed `updatedInput.plan` becomes the plan (tool result \"edited by user\", plan file rewritten, model follows it). Deny: error tool result, model revises", "`r4-plan-from-default`"),
    ("12", "`/compact` with almost no history", "Always a `result`. Empty: synthetic `Error: No messages to compact` + success result. One turn: full compaction", "`r4-compact-empty`"),
    ("13", "`set_model` default; effort; `--fallback-model`", "`default` -> `claude-opus-5-5`. `apply_flag_settings {effortLevel}` -> success, visible in `get_settings`; a bad level also answers success. Fallback model shows nowhere", "`r4-model-effort-fallback`"),
    ("14", "Sandbox + `curl`", "`can_use_tool` `SandboxNetworkAccess` `{host}` with a `WebFetch` `domain:` suggestion and its own uuid `tool_use_id`. Allow: normal result via proxy. Deny: exit 56, `<sandbox_violations>`", "`r4-sandbox-network-allow`, `r4-sandbox-network-deny`"),
    ("15", "`autoAllowBashIfSandboxed:false`", "Bash is back under the permission mode: asks in `manual`, not in `acceptEdits`. With auto-allow, `manual` does not ask either", "`r4-sandbox-noauto`, `r4-sandbox-manual`, `r4-sandbox-manual-noauto`"),
    ("16", "`failIfUnavailable`; sandbox status", "Status: `get_sandbox_dialog`. Failure text: not verified", "`r4-sandbox-required-available`, `r4-sandbox-required-unavailable`"),
    ("17", "Sandbox + linked worktree + `git commit`", "Not verified (work folder lies in a tree the sandbox lets commands write). Recorded: the commit command asks even with the sandbox on; `vcs_state_changed` line", "`r4-sandbox-worktree-linked`, `-plain`, `-linked-off`"),
    ("18", "MCP server that needs sign-in", "`pending`, then `failed` + \"Dynamic Client Registration rejected (HTTP 401)\"; `mcp_reconnect` and `mcp_toggle` on = control error with that text; toggle off = `disabled`", "`r4-mcp-needs-auth`"),
    ("19", "Project `.mcp.json`, headless", "Started with no approval before the first message (`connected`, `source:\"project\"`); ignored with `--strict-mcp-config`", "`r4-project-mcp-project`, `r4-project-mcp-strict`"),
    ("20", "PNG of 1-3 MB", "1.92 MB and 3.00 MB accepted, no error text", "`r4-big-image`"),
]
w("| # | question | answer | fixture |\n|---|---|---|---|")
for row in T:
    w("| %s | %s | %s | %s |" % row)
w("")

w("""## Not verified

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
""")

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES-part2.md")
open(out, "w").write("\n".join(P))
print(out, "written:", sum(len(x) for x in P), "chars")
