"""Builds the part-4 document from the sanitized r6-* fixtures, so every JSON line quoted there is copied from a fixture.

python3 make_notes_part4.py [output file]      (default: $CAPTURE_DIR/NOTES-part4.md)

Numbers and words that the text states about a fixture are checked against it (`check`): the build fails when a
recording no longer says what the text says.
"""
import json
import os
import sys

from notes_lib import (OUT, TASK, block, check, closing, control, cost, flags, fx, has_text, has_tool_use, is_cli, is_tool_result, j, kind, meta,
                       metas, quiet, results, sent, slim, sub, tl)

P = []
w = P.append

ALL = sorted(f[:-len(".ndjson")] for f in os.listdir(os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures")) if f.startswith("r6-"))
# The CLI's total for a session goes on after --resume (part 3, section 14): the budget runs of one label are one
# session, and so are the source session of section 9 and the three runs that resume it from another folder.
RESUMED = [[n for n in ALL if n.startswith("r6-budget-midturn-%s-" % label)] for label in "12"] + [
    ["r6-resume-source"] + ["r6-resume-other-folder-%d" % i for i in (1, 2, 3)]]
TOTAL = (cost([n for n in ALL if not any(n in g for g in RESUMED)])
         + sum(max(r.get("total_cost_usd") or 0.0 for n in g for r in results(n)) for g in RESUMED))
N5 = ["r6-model-while-streaming-1", "r6-model-while-streaming-2"]
for n in ALL:
    check("claude 2.1.311, recorded 2026-11-23." in fx(n)[0]["_note"], "the version and the day of recording of " + n)


def note(name, key):
    """The one driver note of a fixture that has this key: the value of the key."""
    found = meta(name, key)
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0][key]


def stamp(name, key):
    """`_t` of the driver note that has this key (seconds since the process started)."""
    found = [l["_t"] for l in fx(name) if "_meta" in l and key in l["_meta"]]
    check(len(found) == 1, "one note %r in %s" % (key, name))
    return found[0]


def exit_code(name):
    return note(name, "exit_code")


def from_note(key):
    return lambda l: "_meta" in l and key in l["_meta"]


def after_note(name, key):
    """The lines of a fixture from the driver note with this key on."""
    lines = fx(name)
    return lines[[i for i, l in enumerate(lines) if from_note(key)(l)][0]:]


def both(names):
    return "`%s`" % "`, `".join(names)


def secs(values):
    return ", ".join("%.3f" % v for v in values)


w("""# Claude Code CLI stream-json: recorded protocol shapes, part 4

- CLI: `claude` 2.1.311 (the same version as parts 1 to 3; checked with `claude --version` before the runs).
- Recorded: 2026-11-23, macOS, `--model haiku` (`claude-haiku-5-5`), subscription login (`"apiKeySource":"none"`).
- Method, base command, fixture line format and sanitizing: as in part 1 ("Base command", "Files", "Sanitizing") with
  the additions of parts 2 and 3. Fixtures of this part are `fixtures/claude-2.1.311/r6-*.ndjson`; the scripts are
  `drivers/claude-stream-json/r6_*.py` (README there says how to record again).
- List price of the %d recordings (sum of `total_cost_usd` of their `result` lines; a session that several recordings
  resume is counted once): %.2f USD. %.2f USD of it is section 5, the one scenario that leaves haiku on purpose (one
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
""" % (len(ALL), TOTAL, cost(N5)))

# ================================================================= 1
w("## 1. An `interrupt` while a helper (subagent) works\n")
w("Question: (a) a FOREGROUND helper (its `Agent` call still waits): which lines end it, with which status word? (b) a BACKGROUND helper: does it go on, "
  "is it still in `background_tasks_changed`, what ends it later?\n")
N1F = ["r6-interrupt-helper-foreground-1", "r6-interrupt-helper-foreground-2"]
N1B = ["r6-interrupt-helper-background-1", "r6-interrupt-helper-background-2"]
N1I = ["r6-interrupt-helper-idle-1", "r6-interrupt-helper-idle-2"]
V2 = ["sequence", "idle-interrupt", "idle-close-now", "idle-interrupt-again", "idle-outwait"]
N2 = {v: ["r6-shutdown-helper-%s-%d" % (v, i) for i in (1, 2)] for v in V2 + ["foreground-stop"]}
w("Fixtures: %s (a), %s (b, while another turn runs), %s (b, while no turn runs); each scenario twice. Flags: %s. "
  "The recordings of section 2 start the same way and are counted where they show the same thing.\n" % (both(N1F), both(N1B), both(N1I), flags(N1F[0])))


def request_at(name, subtype):
    """Index of the host's first control request of that subtype in a fixture."""
    return [i for i, l in enumerate(fx(name)) if "_sent" in l and l["_sent"].get("request", {}).get("subtype") == subtype][0]


def story(name, subtype):
    """The stdout lines that follow the host's first request of that subtype, as short words, up to the next `result`
    or to the next line the host wrote. A task line says whose task it is: the helper's (`local_agent`) or a Bash
    call's (`local_bash`)."""
    lines = fx(name)
    tasks = {l["task_id"]: {"local_agent": "helper", "local_bash": "bash"}[l["task_type"]] for l in lines if sub(l) == "task_started"}
    out = []
    for l in lines[request_at(name, subtype) + 1:]:
        if "_sent" in l:
            break
        if not is_cli(l) or not quiet(l) or sub(l) in ("status", "task_progress"):
            continue
        if sub(l) in ("task_updated", "task_notification"):
            out.append("%s:%s:%s" % (l["subtype"], tasks[l["task_id"]], l.get("status") or l["patch"].get("status")))
        elif sub(l) == "background_tasks_changed":
            out.append("list:%d" % len(l["tasks"]))
        elif l["type"] == "control_response":
            out.append("answer")
        elif l["type"] in ("user", "assistant"):
            out.append(l["type"] + (" (helper)" if l.get("parent_tool_use_id") else ""))
        else:
            out.append(l["type"] + ("/" + l["subtype"] if l.get("subtype") else ""))
        if l["type"] == "result":
            break
    return out


def bash_calls(name, subtype):
    """The Bash calls that had become tasks of their own before the host's first request of that subtype: how many,
    after a check of what the stream says about them once the request is in."""
    lines, at = fx(name), request_at(name, subtype)
    helpers = {l["task_id"] for l in lines if sub(l) == "task_started" and l["task_type"] == "local_agent"}
    started = [l for l in lines[:at] if sub(l) == "task_started" and l["task_type"] == "local_bash"]
    for l in started:
        check(l["is_backgrounded"] is False and (l.get("parent_task_id") in helpers) == bool(l.get("owned_by_subagent")), "a Bash call's task_started in " + name)
    ids = {l["task_id"] for l in started}
    ended = [l for l in lines[at:] if sub(l) == "task_notification" and l["task_id"] not in helpers]
    check({l["task_id"] for l in ended} == ids and len(ended) == len(ids) and all(l["status"] == "stopped" and l["output_file"] == "" for l in ended),
          "every Bash call that was a task gets one task_notification after the stop, and no other does: " + name)
    check(not [l for l in lines[at:] if sub(l) == "task_updated" and l["task_id"] in ids], "no task_updated for a Bash call in " + name)
    return len(ids)


BASH = "task_notification:bash:stopped"
STOP1 = lambda l: kind(l) in ("sent", "meta", "control_response", "result", "user") or sub(l) in TASK + ("init",) or has_text(l)
w("### (a) The helper's `Agent` call still waits\n")
w("One turn: the model starts a subagent with `run_in_background:false`; the subagent's Bash call runs `echo begin; sleep 127; echo end`. "
  "3 s into that call the host sends `interrupt`, reads for 10 s, sends `stop_task` with the helper's task id, and one more message.\n")
w(block([j(x) for x in sent(N1F[0], "user")[:1]]))
w("The task lines before the interrupt, then every line from the driver's note before the interrupt to the end (first recording):\n")
w(block(tl(N1F[0], lambda l: sub(l) == "task_started", stop=from_note("before_the_interrupt"), text=120)
        + tl(N1F[0], STOP1, start=from_note("before_the_interrupt"), text=120, maxlist=9)))
w("The line that rejects the `Agent` call, with its `tool_result_meta`:\n")
w(block([j(slim([l for l in after_note(N1F[0], "before_the_interrupt") if is_tool_result(l)][0]))]))
T1F = []
for n in N1F:
    before, after = note(n, "before_the_interrupt"), note(n, "after_the_interrupt")
    check(before == {"list": [], "helper_is_backgrounded": False, "sleep_127_alive": True}, "before the interrupt in " + n)
    check(bash_calls(n, "interrupt") == 1 and story(n, "interrupt") == ["answer", BASH, "user", "task_updated:helper:killed", "task_notification:helper:stopped", "user",
                                                                         "result/error_during_execution"], "the lines after the interrupt in " + n)
    check(after["results_in_the_10s_after"] == 1 and not after["sleep_127_alive"] and after["list"] == [], "after the interrupt in " + n)
    rejected = [l for l in after_note(n, "before_the_interrupt") if is_tool_result(l)]
    check(len(rejected) == 1 and rejected[0]["tool_result_meta"][0]["non_execution_kind"] == "user-rejected", "the rejected Agent call in " + n)
    check(results(n)[0]["subtype"] == "error_during_execution" and results(n)[0]["terminal_reason"] == "aborted_tools", "the stopped turn's result in " + n)
    T1F.append(note(n, "seconds_from_the_interrupt_request"))
    check(T1F[-1]["next_init"] is None, "no turn by itself in the 10 s after the interrupt in " + n)
    check(story(n, "stop_task") == ["answer"] and [r.get("result") for r in results(n)][-1] == "alive" and exit_code(n) == 0, "stop_task afterwards and the next turn in " + n)
check(all(t["answer"] < t["task_updated"] < t["result"] < 0.2 for t in T1F), "the order and the times of the lines after the interrupt")
w("""What the adapter can rely on (n=2, the two recordings have the same lines in the same order):
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
- From the request to the `result`: %s s (time stamps of the lines).
- `stop_task` with the helper's task id afterwards answers `{}` and writes nothing. The next turn works.
""" % secs([t["result"] for t in T1F]))

w("### (b) A background helper, while another turn runs\n")
w("Turn 1 starts a subagent with `run_in_background:true` (its Bash call: `echo begin; sleep 43; echo end`) and ends. Turn 2 runs "
  "`echo go; sleep 59` in the foreground. 3 s into that call the host sends `interrupt` and reads for 10 s; then one more message. "
  "The task lines before the interrupt, then every line from the driver's note before the interrupt to the note after it (first recording):\n")
w(block(tl(N1B[0], lambda l: sub(l) == "task_started", stop=from_note("before_the_interrupt"), text=120)
        + tl(N1B[0], STOP1, start=from_note("before_the_interrupt"), stop=from_note("after_the_interrupt"), text=120, maxlist=9)))
HEAD1 = ["task_updated:helper:killed", "task_notification:helper:stopped", "list:0", "answer"]
T1B, MID1 = [], []
for n in N1B + N2["sequence"]:
    k, s1 = bash_calls(n, "interrupt"), story(n, "interrupt")
    check(s1[:4] == HEAD1 and s1[-1] == "result/error_during_execution" and sorted(s1[4:-1]) == sorted(["user", "user", "user (helper)", "user (helper)"] + [BASH] * k),
          "the lines after the interrupt in " + n)
    MID1.append([x for x in s1[4:-1] if x != BASH])
for n in N1B:
    before, after = note(n, "before_the_interrupt"), note(n, "after_the_interrupt")
    check(len(before["list"]) == 1 and before["list"][0]["task_type"] == "local_agent" and before["a_turn_runs"] and before["sleep_43_alive"] and before["sleep_59_alive"],
          "before the interrupt in " + n)
    check(after["results_in_the_10s_after"] == 1 and not after["sleep_43_alive"] and not after["sleep_59_alive"] and after["list"] == [], "after the interrupt in " + n)
    check(not meta(n, "the_helper_ended_by_itself") and note(n, "at_the_end")["results"] == 3 and exit_code(n) == 0, "the end of " + n)
    T1B.append(note(n, "seconds_from_the_interrupt_request"))
check(all(x == ["user", "user", "user (helper)", "user (helper)"] for x in MID1), "the turn's two user lines come before the helper's two")
w("### (b) A background helper, while NO turn runs\n")
w("The same turn 1. No second turn: 3 s into the helper's Bash call the host sends `interrupt` and reads for 10 s; then one more message. "
  "The task lines before the interrupt, then every line from the driver's note before the interrupt to the note after it (first recording):\n")
w(block(tl(N1I[0], lambda l: sub(l) == "task_started", stop=from_note("before_the_interrupt"), text=120)
        + tl(N1I[0], STOP1, start=from_note("before_the_interrupt"), stop=from_note("after_the_interrupt"), text=120, maxlist=9)))
T1I = []
for n in N1I + N2["idle-interrupt"]:
    k, s1 = bash_calls(n, "interrupt"), story(n, "interrupt")
    check(s1[:4] == HEAD1 and sorted(s1[4:]) == sorted(["user (helper)", "user (helper)"] + [BASH] * k), "the lines after the interrupt in " + n)
for n in N1I:
    before, after = note(n, "before_the_interrupt"), note(n, "after_the_interrupt")
    check(len(before["list"]) == 1 and not before["a_turn_runs"] and before["sleep_43_alive"], "before the interrupt in " + n)
    check(after["results_in_the_10s_after"] == 0 and not after["sleep_43_alive"] and after["list"] == [], "after the interrupt in " + n)
    check(not meta(n, "the_helper_ended_by_itself") and note(n, "at_the_end")["results"] == 2 and exit_code(n) == 0, "the end of " + n)
    T1I.append(note(n, "seconds_from_the_interrupt_request"))
check(all(t["task_updated"] <= t["answer"] < 0.2 for t in T1B + T1I), "the helper's end lines come before the answer, within 0.2 s")
# Whose call a `task_notification` after an interrupt names: a helper's line has the id of the helper's `Agent` call,
# a Bash call's line the id of that Bash call.
for n in N1F + N1B + N1I + N2["sequence"] + N2["idle-interrupt"]:
    lines = fx(n)
    calls = {b["id"]: b["name"] for l in lines if l.get("type") == "assistant" for b in l["message"]["content"] if b["type"] == "tool_use"}
    tasks = {l["task_id"]: {"local_agent": "Agent", "local_bash": "Bash"}[l["task_type"]] for l in lines if sub(l) == "task_started"}
    ended = [l for l in lines[request_at(n, "interrupt"):] if sub(l) == "task_notification"]
    check(ended and all(calls[l["tool_use_id"]] == tasks[l["task_id"]] for l in ended), "the tool_use_id of the task_notification lines in " + n)
# Every recording of sections 1 and 2 stops something: in how many had a Bash call become a task before the stop?
STOPS = [(n, "interrupt") for n in N1F + N1B + N1I + N2["sequence"] + N2["idle-interrupt"]] + [
    (n, "stop_task") for v in ("idle-close-now", "idle-interrupt-again", "idle-outwait", "foreground-stop") for n in N2[v]]
WITH_BASH = sum(bash_calls(n, r) > 0 for n, r in STOPS)
w("""What the adapter can rely on (with a turn: n=4, the two recordings here and the two `sequence` recordings of section 2;
without a turn: n=4, the two here and the two `idle-interrupt` recordings of section 2; the eight agree):
- A background helper does NOT go on after an `interrupt`. The interrupt ends it, whether a turn runs or not:
  `task_updated` (`patch.status:"killed"`), `task_notification` (`"status":"stopped"`), `background_tasks_changed`
  with `[]`, and only THEN the answer to `interrupt`. From the request to the answer: %s s.
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
  task yet depends on how long it has run: in %d of the %d recordings of sections 1 and 2 at least one call was, in
  the others the stop came first and no such line was written. The builder of this document checks for all %d that
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
""" % (secs([t["answer"] for t in T1B + T1I]), WITH_BASH, len(STOPS), len(STOPS)))

# ================================================================= 2
w("## 2. The stop of a helper inside a shutdown; `stop_task` on a foreground helper\n")
w("Question: as one run: `interrupt`, `stop_task` on the background helper, the turn Claude Code then starts by itself, stdin close. What must the host do "
  "so that the process exits, how long does it take, is anything left running? And `stop_task` on a helper whose `Agent` call still waits.\n")
w("Fixtures: `r6-shutdown-helper-<variant>-1` and `-2` for the variants `sequence`, `idle-interrupt`, `idle-close-now`, `idle-interrupt-again`, `idle-outwait`, "
  "`foreground-stop` (each twice). Flags: %s. The CLI is the leader of its own process group, as the app starts it.\n" % flags(N2["sequence"][0]))
w("All variants but the last: turn 1 starts a subagent in the background (its Bash call: `echo begin; sleep 139; echo end`) and ends. "
  "The host waits until it sees the `sleep 139` process, and 2 s more. Then:\n")
DID = {"sequence": "a turn runs (`echo go; sleep 67`): `interrupt`, the stopped turn's `result`, `stop_task` on the helper, stdin closed",
       "idle-interrupt": "no turn runs: `interrupt`, `stop_task` on the helper, stdin closed",
       "idle-close-now": "no turn runs: `stop_task` on the helper, stdin closed right after its answer",
       "idle-interrupt-again": "no turn runs: `stop_task` on the helper, wait for the `system/init` of the self-started turn, `interrupt`, that turn's `result`, stdin closed",
       "idle-outwait": "no turn runs: `stop_task` on the helper, the self-started turn's `result`, stdin closed"}
SEQ2 = lambda l: kind(l) in ("sent", "meta", "control_response", "result", "user") or sub(l) in TASK + ("init",) or has_text(l)
w("### `sequence`: a turn runs\n")
w("Every line from the driver's note before the interrupt (first recording):\n")
w(block(tl(N2["sequence"][0], SEQ2, start=from_note("before_the_stop"), text=120, maxlist=9)))
w("### `idle-close-now`: `stop_task` with no turn running, stdin closed at once\n")
w("Every line from the driver's note before `stop_task` (first recording):\n")
w(block(tl(N2["idle-close-now"][0], SEQ2, start=from_note("before_the_stop"), text=120, maxlist=9)))
_late = meta(N2["idle-close-now"][0], "before_stdin_is_closed")[0]
_before = fx(N2["idle-close-now"][0])[:[i for i, l in enumerate(fx(N2["idle-close-now"][0])) if from_note("before_stdin_is_closed")(l)][0]]
check(_late["seconds_from_stop_task"]["init_of_a_turn"] is None and len(_late["before_stdin_is_closed"]["lines_since_the_first_request"]) == 4
      and any(sub(l) == "init" for l in _before[[i for i, l in enumerate(_before) if "_sent" in l][-1]:]), "the late note of idle-close-now")
w("The note `before_stdin_is_closed` above is older than its time stamp: the driver listed the lines, then looked for two "
  "processes, then wrote the note, and the `user`, `user` and `system/init` lines came in between (they are in the file before "
  "the note; its `init_of_a_turn` is null and its list has 4 lines). Stdin was closed after that `init`. Part 5 (\"Notes of part 4 "
  "written late\") lists every note of part 4 this touches; the script is fixed.\n")
w("### `idle-interrupt-again`: the self-started turn is interrupted\n")
w("From `stop_task` on (first recording):\n")
w(block(tl(N2["idle-interrupt-again"][0], SEQ2, start=lambda l: "_sent" in l and l["_sent"].get("request", {}).get("subtype") == "stop_task", text=120, maxlist=9)))


def exit_after_close(name):
    return stamp(name, "process_exited") - stamp(name, "closing_stdin")


w("All ten recordings (the time from stdin close to the exit is the difference of the two notes' stamps):\n")
w("| variant | what the host did | lines after stdin was closed | exit after stdin close (s), run 1 / run 2 | exit code | `sleep` processes left |")
w("|---|---|---|---|---|---|")
E2 = {}
for v in V2:
    rows = []
    for n in N2[v]:
        b, a = note(n, "before_the_stop"), note(n, "after_the_exit")
        check(b["sleep_139_alive"] and b["sleep_67_alive"] == (v == "sequence") and len(b["list"]) == 1, "the commands ran before the stop in " + n)
        check(not a["sleep_139_alive"] and not a["sleep_67_alive"], "nothing left after " + n)
        rows.append((a["lines_after_stdin_was_closed"], exit_after_close(n), exit_code(n)))
    check(rows[0][0] == rows[1][0] and rows[0][2] == rows[1][2], "the two recordings of %s end the same way" % v)
    E2[v] = rows
    w("| `%s` | %s | %s | %.2f / %.2f | %d | none |" % (v, DID[v], "`%s`" % "`, `".join(rows[0][0]) if rows[0][0] else "none", rows[0][1], rows[1][1], rows[0][2]))
w("")
check([E2[v][0][2] for v in V2] == [1, 0, 0, 1, 0], "exit codes of the shutdown variants")
check(E2["idle-close-now"][0][0] == ["assistant", "assistant", "result/success"], "the self-started turn runs to its end after stdin was closed")
check(all("system/init" in story(n, "stop_task") for n in N2["idle-close-now"]), "the self-started turn had begun when stdin was closed")
check(all(not E2[v][0][0] for v in V2 if v != "idle-close-now"), "no line after stdin close in the other variants")
STILL2 = 0  # recordings in which the helper's `sleep` was still seen right after the answer to the interrupt
for n in N2["sequence"] + N2["idle-interrupt"]:
    a = note(n, "after_the_interrupt")
    check(a["list"] == [], "the interrupt ended the helper in " + n)
    STILL2 += a["sleep_139_alive"]
    check(story(n, "stop_task") == ["answer"], "stop_task after the interrupt writes nothing but its answer in " + n)
    check(len(results(n)) == (2 if "sequence" in n else 1), "no turn by itself in " + n)
for v in ("idle-close-now", "idle-interrupt-again", "idle-outwait"):
    for n in N2[v]:
        check(bash_calls(n, "stop_task") == 0 and story(n, "stop_task")[:4] == HEAD1, "stop_task on the background helper in " + n)
SELF2 = [r for v in ("idle-close-now", "idle-outwait") for n in N2[v] for r in results(n)[1:]]
check(len(SELF2) == 4 and all(r["subtype"] == "success" and r.get("origin", {}).get("kind") == "task-notification" for r in SELF2), "the self-started turns")
AGAIN2 = [results(n)[1] for n in N2["idle-interrupt-again"]]
check(all(r["subtype"] == "error_during_execution" and r["terminal_reason"] == "aborted_streaming" for r in AGAIN2), "the interrupted self-started turns")
INIT2 = [note(n, "seconds_from_stop_task")["init_of_a_turn"] for v in ("idle-interrupt-again", "idle-outwait") for n in N2[v]]
check(all(t is not None and t < 2 for t in INIT2), "the self-started turn begins within 2 s of stop_task")
CLOSED2 = [stamp(n, "closing_stdin") - fx(n)[request_at(n, "stop_task")]["_t"] for n in N2["idle-close-now"]]
T2SEQ = [note(n, "seconds_from_the_interrupt") for n in N2["sequence"]]
check(all(t["result"] < 2 for t in T2SEQ), "the stopped turn's result within 2 s")
w("""What the adapter can rely on (n=2 per variant):
- With a turn running (`sequence`), the order of the question needs no extra step: the `interrupt` ends the turn AND the
  background helper (section 1), the stopped turn's `result` came %s s after the request, `stop_task` on the helper's
  id then answers `{}` and writes nothing, no turn starts by itself, and after stdin close the process was gone in
  %.2f / %.2f s with exit code 1 (the last `result` was the interrupted one). Nothing was left running.
- With no turn running, an `interrupt` does the same to the helper (`idle-interrupt`): helper ended, no turn by itself,
  exit %.2f / %.2f s after stdin close, exit code 0.
- `stop_task` on the helper with NO interrupt before it starts a turn by itself (part 3, section 1): its
  `system/init` came %s s after the request in the four recordings that waited for it (four values are no upper
  bound: a host cannot tell "no turn will come" from "it has not come yet" by waiting a moment). What the host does
  next decides the rest:
  - stdin closed at once (`idle-close-now`; %s s after the request, when the turn's `system/init` had come): the
    process does NOT exit before that turn is over. The turn runs to its end after stdin was closed (an answer, a `result` with
    `origin.kind:"task-notification"`), and only then the process exits: %.2f / %.2f s after stdin close, exit code
    0. So closing stdin does not stop the self-started turn; the host pays for it and waits for it.
  - `interrupt` when that turn's `system/init` came (`idle-interrupt-again`): the answer, a `user` line
    `[Request interrupted by user]`, the turn's `result` (`error_during_execution`, `terminal_reason:"aborted_streaming"`);
    exit %.2f / %.2f s after stdin close, exit code 1.
  - read to the turn's `result` (`idle-outwait`): exit %.2f / %.2f s after stdin close, exit code 0.
- In all ten recordings the scenario's `sleep` processes ran before the stop and none was alive after the exit;
  nothing had to be signalled. The task lines come a moment before the helper's process is dead: in %d of the 4
  recordings with an interrupt the `sleep 139` was still seen when the driver looked right after the answer, and it
  was gone at the next look.
- For a shutdown: send `interrupt` first, also when no turn runs. It ends every helper and starts no turn. Then
  `stop_task` for the background COMMANDS (they survive an interrupt: part 3, section 4), then close stdin. A
  `stop_task` on a helper is not needed after an interrupt; without an interrupt it costs one more turn.
""" % (secs([t["result"] for t in T2SEQ]), E2["sequence"][0][1], E2["sequence"][1][1], E2["idle-interrupt"][0][1], E2["idle-interrupt"][1][1], secs(INIT2),
       secs(CLOSED2), E2["idle-close-now"][0][1], E2["idle-close-now"][1][1], E2["idle-interrupt-again"][0][1], E2["idle-interrupt-again"][1][1],
       E2["idle-outwait"][0][1], E2["idle-outwait"][1][1], STILL2))

w("### `foreground-stop`: `stop_task` on a helper whose `Agent` call still waits\n")
w("One turn: a subagent with `run_in_background:false` (its Bash call: `echo begin; sleep 139; echo end`). 3 s into that call the host sends `stop_task` "
  "with the helper's task id. The task lines before it, then every line from the driver's note before it to the note after it (first recording):\n")
NF = N2["foreground-stop"]
w(block(tl(NF[0], lambda l: sub(l) == "task_started", stop=from_note("before_stop_task"), text=120)
        + tl(NF[0], SEQ2, start=from_note("before_stop_task"), stop=from_note("after_stop_task"), text=160, maxlist=9)))
T2F = []
for n in NF:
    before, after = note(n, "before_stop_task"), note(n, "after_stop_task")
    check(before == {"list": [], "helper_is_backgrounded": False, "sleep_139_alive": True}, "before stop_task in " + n)
    s2 = story(n, "stop_task")
    check(bash_calls(n, "stop_task") == 1 and s2[:6] == ["task_updated:helper:killed", "task_notification:helper:stopped", "answer", BASH, "user (helper)", "user"]
          and set(s2[6:-1]) == {"assistant"} and s2[-1] == "result/success", "the lines after stop_task in " + n)
    check(after["turn_result_within_30s"] and not after["sleep_139_alive"] and after["list"] == [], "after stop_task in " + n)
    rejected = [l for l in fx(n) if is_tool_result(l) and not l.get("parent_tool_use_id") and "[Request interrupted by user for tool use]" in json.dumps(l)]
    check(len(rejected) == 1 and rejected[0]["tool_result_meta"][0]["non_execution_kind"] == "interrupted", "the Agent call's result in " + n)
    check(results(n)[0]["subtype"] == "success" and results(n)[-1].get("result") == "alive" and exit_code(n) == 0, "the turn goes on in " + n)
    T2F.append(note(n, "seconds_from_the_stop_task_request"))
check(all(t["task_updated"] <= t["answer"] < 0.2 for t in T2F), "the helper's task lines come before the answer")
w("""What the adapter can rely on (n=2, the two recordings agree line for line up to the model's answer):
- `stop_task {"task_id": <the helper's id>}` works on a FOREGROUND helper too: `task_updated`
  (`patch.status:"killed"`), `task_notification` (`"status":"stopped"`), then the answer `{}` (%s s after the
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
""" % secs([t["answer"] for t in T2F]))

# ================================================================= 3
w("## 3. A helper that fails by itself; a background command killed by a signal from outside\n")
w("Question: the status word of the `task_notification` of a helper that fails by itself; the word and the list line when a background command is killed by a signal.\n")
N3H = ["r6-fails-helper-background", "r6-fails-helper-foreground"]
N3S = ["r6-fails-signal-term", "r6-fails-signal-kill", "r6-fails-signal-child-kill"]
w("Fixtures: %s (a custom agent whose model name does not exist: its first model request is refused; flags: %s), %s (flags: %s).\n"
  % (both(N3H), flags(N3H[0]), both(N3S), flags(N3S[0])))
w("### The helper\n")
w("Started in the background. Task lines, the helper's own lines, results (every line of the kind up to the end of the turn the CLI starts by itself):\n")
HELP3 = lambda l: sub(l) in TASK or kind(l) in ("result", "stderr") or (kind(l) in ("assistant", "user") and l.get("parent_tool_use_id"))
w(block(tl(N3H[0], HELP3, text=200, maxstr=420)))
w("Started in the foreground (`run_in_background:false`): the same lines, and the result of the `Agent` call:\n")
w(block(tl(N3H[1], lambda l: HELP3(l) or (is_tool_result(l) and not l.get("parent_tool_use_id")), text=200, maxstr=420)))
for n in N3H:
    h = note(n, "the_helper")
    upd = [l for l in fx(n) if sub(l) == "task_updated"]
    check(h["task_notification_status"] == "failed" and len(upd) == 1 and upd[0]["patch"]["status"] == "failed" and "error" in upd[0]["patch"], "the failed helper of " + n)
    check(h["list"] == [] and h["is_backgrounded"] == ("background" in n) and h["results"] == (2 if "background" in n else 1), "the end of " + n)
check(results(N3H[0])[1].get("origin", {}).get("kind") == "task-notification", "a self-started turn after the failed background helper")
OWN3 = [[(l["type"], l["message"].get("model")) for l in fx(n) if kind(l) in ("assistant", "user") and l.get("parent_tool_use_id")] for n in N3H]
check(OWN3 == [[("assistant", "<synthetic>")], [("user", None)]], "the helper's own lines in the two runs")
check(sum(1 for n in N3H for l in fx(n) if "_stderr" in l and "unrecognized_model" in l["_stderr"]) == 2 and all(len([l for l in fx(n) if "_stderr" in l]) == 1 for n in N3H),
      "one stderr line in each helper run")
check(any(is_tool_result(l) and not l.get("parent_tool_use_id") and "Agent terminated early due to an API error" in json.dumps(l)
          and l["message"]["content"][0].get("is_error") for l in fx(N3H[1])), "the Agent call's error result")
w("### The command\n")
w("Turn 1 starts `echo begin; sleep 131; echo end` with `run_in_background:true`. Two processes then have the command in their command line: the shell that the CLI "
  "started for it, and the `sleep` (a child of that shell). The driver sends one signal. The driver's notes and the task lines after it, for SIGTERM to the shell:\n")
SIG3 = lambda l: sub(l) in TASK or kind(l) in ("meta", "result")
w(block(tl(N3S[0], SIG3, start=from_note("before_the_signal"), text=160, maxlist=9)))
w("| signal | sent to | `task_updated` `patch.status` | `task_notification` `status` | `summary` | list after | the shell | the `sleep` |")
w("|---|---|---|---|---|---|---|---|")
S3 = {}
for n in N3S:
    sig, after = note(n, "signal_sent"), note(n, "after_the_signal")
    upd = [l for l in fx(n) if sub(l) == "task_updated"][-1]
    tn = [l for l in fx(n) if sub(l) == "task_notification"][-1]
    check(sig["delivered"] and after["list"] == [] and after["task_notification_status"] == tn["status"], "the signal of " + n)
    check(results(n)[-1].get("origin", {}).get("kind") == "task-notification" and exit_code(n) == 0, "a self-started turn after the signal in " + n)
    S3[n] = (upd["patch"]["status"], tn["status"], tn["summary"], after["shell_alive"], after["sleep_alive"])
    w("| %s | %s | `%s` | `%s` | `%s` | `[]` | %s | %s |" % (sig["signal"], sig["to"], upd["patch"]["status"], tn["status"], tn["summary"],
                                                          "alive" if after["shell_alive"] else "gone", "alive" if after["sleep_alive"] else "gone"))
w("")
check([S3[n][:2] for n in N3S] == [("failed", "failed"), ("failed", "failed"), ("completed", "completed")], "status words after the signals")
check([S3[n][3:] for n in N3S] == [(False, True), (False, True), (False, False)], "what the signals left alive")
check(S3[N3S[0]][2].endswith("failed with exit code 144") and S3[N3S[1]][2].endswith("failed with exit code 1") and S3[N3S[2]][2].endswith("completed (exit code 0)"),
      "the summaries after the signals")
T3 = [note(n, "seconds_from_the_signal")["task_notification"] for n in N3S]
check(all(t < 0.1 for t in T3), "the end lines come within 0.1 s of the signal")
w("""What the adapter can rely on (n=1 per row; the two helper runs agree on the word):
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
  SIGTERM and `failed with exit code 1` after SIGKILL. The lines came %s s after the signal.
- The list line after it is `[]`, and the CLI starts a turn by itself, as after any end of a background command.
- THE LIST SAYS NOTHING ABOUT WHAT THE COMMAND LEFT BEHIND: in both shell runs the `sleep 131` child was still alive
  after the list was empty (the driver killed it at the end). "Not listed" means "the CLI no longer tracks it", not
  "none of its processes run".
- SIGKILL to the `sleep` child alone: the shell goes on (`echo end`), the command ends as `completed`
  (`completed (exit code 0)`): the death of a child is not visible at all.
""" % secs(T3))

# ================================================================= 4
w("## 4. `apply_flag_settings` with `effortLevel:null` as the FIRST such request on a `--effort high` process\n")
w("Question: what does `get_settings` say after it? (Part 3, section 7, sent `null` after an `effortLevel:\"low\"`.)\n")
N4 = ["r6-effort-null-first-fresh", "r6-effort-null-first-after-turn"]
w("Fixtures: `r6-effort-null-first-fresh` (nothing before the `null` but `initialize` and `get_settings`), `r6-effort-null-first-after-turn` (one turn before it). "
  "Flags: %s.\n" % flags(N4[0]))
w("`fresh`: every control request after `initialize`, with its answer:\n")
w(block(control(N4[0])[2:]))
w("The driver's notes of both runs (`get_settings` read after each step):\n")
w("| run | step | `applied.effort` | `effective.effortLevel` | sources that name `effortLevel` |")
w("|---|---|---|---|---|")
for n in N4:
    for m in meta(n, "step"):
        w("| `%s` | %s | `%s` | %s | %s |" % (n[len("r6-effort-null-first-"):], m["step"], m["applied_effort"],
                                         "`%s`" % m["effective_effortLevel"] if m["effective_effortLevel"] != "(no such key)" else "no such key",
                                         ", ".join("`%s`" % x for x in m["sources_that_name_effortLevel"]) or "none"))
w("")
check([m["applied_effort"] for m in meta(N4[0], "step")] == ["high", "medium", "medium", "high"], "applied.effort of the fresh run")
check([m["applied_effort"] for m in meta(N4[1], "step")] == ["high", "high", "medium", "medium", "high"], "applied.effort of the after-turn run")
check(all(m["sources_that_name_effortLevel"] == (["flagSettings"] if m["step"] == "after effortLevel high" else []) for n in N4 for m in meta(n, "step")),
      "which steps have a source for effortLevel")
w("""What the adapter can rely on (n=2 processes):
- `apply_flag_settings {"settings":{"effortLevel":null}}` as the FIRST such request on a process started with
  `--effort high` takes the start flag's level away: `applied.effort` goes from `high` to `medium` at once, before any
  turn and after one, and stays `medium` through the next turn. The answer is `{"subtype":"success"}` with no body.
- So `null` means "the default" (`medium`), never "back to the start flag", whether or not a level was applied before
  (part 3 had it after `low`). A live request can take an effort level away from a process that was started with one.
- `effortLevel:"high"` after it gives `high` again, now as a flag setting (`effective.effortLevel:"high"`,
  a `flagSettings` source); the start flag's `high` had neither.
- Still not visible in any line: the effort a model request is sent with (part 3, section 7).
""")

# ================================================================= 5
w("## 5. `set_model` while an answer streams\n")
w("Question: `set_model` sent while an answer streams and no tool call runs: the `message.model` of the stream's remaining lines, of the next turn, and the `modelUsage` of the result.\n")
w("Fixtures: %s (the same scenario twice). Flags: %s.\n" % (both(N5), flags(N5[0])))
w("Turn 1 (on haiku) asks for the numbers from 1 to 400. When the first text delta of the answer has come the host sends `set_model sonnet`. "
  "Turn 2 is one tiny turn; then `set_model haiku` and turn 3. Requests, init lines, assistant lines and the driver's note per turn (first recording):\n")
w(block(tl(N5[0], lambda l: (kind(l) == "sent" and l["_sent"].get("request", {}).get("subtype") not in ("get_settings", "initialize"))
           or (kind(l) == "control_response" and "model" in (l["response"].get("response") or {})) or sub(l) == "init" or kind(l) == "assistant"
           or (kind(l) == "user" and l.get("isReplay")) or (kind(l) == "meta" and "turn" in l["_meta"]), text=40, maxlist=9)))
D5 = []
for n in N5:
    t = {m["turn"]: m for m in meta(n, "turn")}
    a, b = t["turn 1"]["before_the_set_model_answer"], t["turn 1"]["after_the_set_model_answer"]
    check(t["turn 1"]["init_model"] == ["claude-haiku-5-5"] and a["stream_message_start_model"] == ["claude-haiku-5-5"] and b["stream_message_start_model"] == [],
          "one model request in turn 1 of " + n)
    check(b["text_deltas"] > 50 and set(b["assistant_message_model"]) == {"claude-haiku-5-5"}, "the rest of the stream of turn 1 in " + n)
    check(t["turn 1"]["result_modelUsage_keys"] == [["claude-haiku-5-5"]], "modelUsage of turn 1 in " + n)
    check(t["turn 2"]["init_model"] == ["claude-sonnet-5-5"] and t["turn 2"]["assistant_message_model"] == ["claude-sonnet-5-5"]
          and t["turn 2"]["result_modelUsage_keys"] == [["claude-haiku-5-5", "claude-sonnet-5-5"]]
          and t["turn 2"]["result_model_usage_since_last_result_keys"] == [["claude-sonnet-5-5"]], "turn 2 of " + n)
    check(t["turn 3"]["init_model"] == ["claude-haiku-5-5"] and t["turn 3"]["assistant_message_model"] == ["claude-haiku-5-5"], "turn 3 of " + n)
    D5.append(b["text_deltas"])
w("""What the adapter can rely on (n=2, the two recordings agree):
- A `set_model` that arrives while an answer streams does not touch that answer. The request that is in flight goes
  on: %s text deltas came after the answer to `set_model`, there was no second `message_start`, and the
  `assistant` line that closes the answer says `claude-haiku-5-5`. The turn's `result.modelUsage` has haiku only.
- `set_model` itself is answered at once (`{"model":"claude-sonnet-5-5"}`), with the `isReplay` `user` line before it
  (as in part 3), and `get_settings.applied.model` says sonnet while the haiku answer still streams.
- The NEXT turn is on the new model: `system/init.model`, `message.model` and a new key in `result.modelUsage`
  (`model_usage_since_last_result` has sonnet alone).
- Together with part 3, section 8 (a switch during a TOOL CALL shows on the next assistant line of the same turn):
  the switch takes effect at the next model REQUEST. A line on the old model after the switch is the request that
  was already in flight, and at most that one request.
""" % " and ".join(str(d) for d in D5))

# ================================================================= 6
w("## 6. Sandbox network: a host that was DENIED once\n")
w("Question: after the network request for a host was denied, is the next command to the same host asked about again in the same process?\n")
N6 = ["r6-sandbox-network-deny-1", "r6-sandbox-network-deny-2"]
w("Fixtures: %s (the same scenario twice). Flags: %s. The capture folder was outside the CLI's own temp folder.\n" % (both(N6), flags(N6[0])))
w("Five turns, one command each. Turns 1 to 4: `curl -sI https://example.com`; the request for the host is denied in turn 1, denied again in turn 2, "
  "allowed in turn 3. Turn 5: `curl -sI https://example.org` (allowed). Every `can_use_tool` request with its answer, the first line of each tool result, "
  "and the driver's note per turn (first recording):\n")


def asks6(l):
    return (kind(l) == "control_request" or (kind(l) == "sent" and l["_sent"].get("type") == "control_response") or (kind(l) == "meta" and "turn" in l["_meta"])
            or is_tool_result(l))


w(block(tl(N6[0], asks6, maxlist=9, maxstr=150)))
for n in N6:
    turns = meta(n, "turn")
    check([[r[1] for r in t["can_use_tool_requests"]] for t in turns] == [["example.com"], ["example.com"], ["example.com"], [], ["example.org"]], "requests per turn of " + n)
    check([t["answers"] for t in turns] == [["deny"], ["deny"], ["allow"], [], ["allow"]] and all(t["tool_calls_of_the_turn"] == 1 for t in turns), "answers per turn of " + n)
    firsts = [json.dumps(l["message"]["content"][0]["content"]) for l in fx(n) if is_tool_result(l)]
    check([("403 user denied" in x) for x in firsts] == [True, True, False, False, False], "which commands got the network error in " + n)
    for l in fx(n):
        r = (l.get("response", {}).get("response") or {}) if l.get("type") == "control_response" else {}
        if "restrictions" in r:
            check(r["restrictions"]["network_allowed_domains"] == [], "network_allowed_domains stays empty in " + n)
        if "state" in r:
            check(not [x for x in r["state"].get("rules", []) if "example" in json.dumps(x)], "no rule for the host in " + n)
w("""What the adapter can rely on (n=2 processes, two "next commands" after a deny in each):
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
""")

# ================================================================= 7
w("## 7. MCP: `mcp_reconnect` on a connected server; a stdio server whose process is killed\n")
w("Question: is a connected server started again by `mcp_reconnect`? When the process of a stdio server is killed mid-session: does `mcp_status` change by itself, "
  "is there a line, what does the next call of its tool return?\n")
N7 = ["r6-mcp-live-server-status-first", "r6-mcp-live-server-call-first"]
w("Fixtures: %s. Flags: %s. The server `ping-live` (the drivers folder's `mcp_ping_server.py`) writes a line with its pid each time it starts; "
  "the driver's notes say how often it was started and which of those processes live.\n" % (both(N7), flags(N7[0])))
w("`status-first`: `mcp_status`, `mcp_reconnect`, `mcp_status`, a turn that calls the tool; the driver kills the server process (SIGKILL) and reads for 8 s; "
  "`mcp_status`, a turn that calls the tool, `mcp_status`, `mcp_reconnect`, `mcp_status`, a turn. Every control request with its answer, the driver's notes, "
  "the init lines and the tool results:\n")
SEQ7 = lambda l: (kind(l) in ("sent", "control_response", "meta") and not (kind(l) == "sent" and l["_sent"].get("type") != "control_request")
                  and not (kind(l) == "meta" and "turn" in l["_meta"])) or sub(l) == "init" or is_tool_result(l)
w(block(tl(N7[0], SEQ7, maxlist=9, maxstr=200)))
w("`call-first` is the same without the `mcp_status` between the kill and the turn. From the kill to the `mcp_status` after the turn:\n")
w(block(tl(N7[1], SEQ7, start=from_note("killed"), stop=lambda l: "_meta" in l and l["_meta"].get("after") == "mcp_reconnect after the kill", maxlist=9, maxstr=200)[:-2]))
for n in N7:
    notes = [l["_meta"] for l in fx(n) if "_meta" in l and "server_starts_so_far" in l["_meta"] and "turn" not in l["_meta"]]
    by = {m.get("after"): m for m in notes if "after" in m}
    check(by["start + 3 s"]["server_processes_alive"] == [True] and by["mcp_reconnect on the connected server"]["server_processes_alive"] == [False, True],
          "mcp_reconnect on a connected server starts it again in " + n)
    check(note(n, "stdout_lines_in_the_8s_after_the_kill") == [] and not meta(n, "stdout_lines_in_the_8s_after_the_kill")[0]["server_process_alive"], "the 8 s after the kill in " + n)
    turns = {m["turn"]: m for m in meta(n, "turn")}
    check(all(m["ok"] and m["tool_results"] == ['[{"type": "text", "text": "pong"}]'] and m["init_mcp_servers"] == [{"name": "ping-live", "status": "connected", "source": "dynamic"}]
              for m in turns.values()), "every call of the tool returns pong in " + n)
    check([turns[k]["server_starts_so_far"] for k in ("after the reconnect", "after the kill", "after the second reconnect")] == [2, 3, 4], "the start counter in " + n)
    st = [r["response"]["mcpServers"][0]["status"] for l in fx(n) if l.get("type") == "control_response" for r in [l["response"]] if "mcpServers" in (r.get("response") or {})]
    check(set(st) == {"connected"} and len(st) == (5 if n == N7[0] else 4), "every mcp_status says connected in " + n)
    check([r["subtype"] for l in fx(n) if l.get("type") == "control_response" for r in [l["response"]] if "response" not in r] == ["success", "success"], "the two mcp_reconnect answers in " + n)
    check(exit_code(n) == 0, "exit code of " + n)
m7 = [l["_meta"] for l in fx(N7[0]) if "_meta" in l and "server_starts_so_far" in l["_meta"] and "turn" not in l["_meta"]]
i7 = [i for i, m in enumerate(m7) if m.get("after") == "the server process was killed 8 s ago"][0]
check(m7[i7]["server_processes_alive"] == [False, False] and m7[i7 + 1].get("after_the_mcp_status_answer") and m7[i7 + 1]["server_processes_alive"] == [False, False],
      "mcp_status says connected while no server process lives, and does not start one")
w("""What the adapter can rely on (n=2):
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
""")

# ================================================================= 8
w("## 8. A project `.mcp.json` server with the app's flags and the REAL login\n")
w("Question: one tool call each with `enableAllProjectMcpServers:false`, with an `enabledMcpjsonServers` list that does not name the server, and with "
  "`disabledMcpjsonServers` naming it (all through `--settings`); and with `--setting-sources user` and no `--strict-mcp-config`: is the server loaded, can its tool be called? "
  "Added after a review: the same with `--setting-sources user,local` when the folder's own `.claude/settings.local.json` turns the project's servers on.\n")
V8 = ["enable-all-false", "enabled-other", "disabled", "user-only", "user-only-home", "user-local-home", "user-local-file", "user-local-file-home-1", "user-local-file-home-2"]
N8 = ["r6-project-mcp-" + v for v in V8]
FILE8 = [n for n in N8 if "-user-local-file" in n]
w("Fixtures: `r6-project-mcp-enable-all-false`, `-enabled-other`, `-disabled` (real login; %s), `-user-only` (real login; %s), `-user-only-home` and "
  "`-user-local-home` (a throw-away HOME, no login), `-user-local-file` (real login; %s), `-user-local-file-home-1` and `-2` (a throw-away HOME, no login), "
  "and the two checks `-hooks-on`, `-hooks-off`. No run has `--strict-mcp-config`. "
  "The work folder's `.mcp.json` defines `ping-project`.\n" % (flags(N8[0]), flags(N8[3]), flags(N8[6])))
w("`enable-all-false`: the driver's notes, the init line, the request and the tool result:\n")
LOAD8 = lambda l: sub(l) == "init" or (kind(l) == "meta" and not closing(l)) or kind(l) == "control_request" or is_tool_result(l)
w(block(tl(N8[0], LOAD8, maxlist=9)))
w("`user-only` (the user's own settings are the only source), the same lines:\n")
w(block(tl(N8[3], LOAD8, maxlist=9)))
w("`user-local-file-home-1` (`--setting-sources user,local`, no login): the lines of the `SessionStart` hook, the init line and the driver's notes. "
  "One note has the folder's `.claude/settings.local.json` (the driver wrote the file before the start and the note 3 s after it):\n")
w(block(tl(FILE8[1], lambda l: LOAD8(l) or sub(l) in ("hook_started", "hook_response"), maxlist=9, maxstr=700)))
NO_HOME = "none (throw-away HOME)"
SET8 = {"enable-all-false": ("user,project", '`{"enableAllProjectMcpServers":false}`', "real"),
        "enabled-other": ("user,project", '`{"enabledMcpjsonServers":["another-server"]}`', "real"),
        "disabled": ("user,project", '`{"disabledMcpjsonServers":["ping-project"]}`', "real"),
        "user-only": ("user", "none", "real"), "user-only-home": ("user", "none", NO_HOME), "user-local-home": ("user,local", "none", NO_HOME),
        "user-local-file": ("user,local", "none", "real"), "user-local-file-home-1": ("user,local", "none", NO_HOME),
        "user-local-file-home-2": ("user,local", "none", NO_HOME)}
w("| run | `--setting-sources` | the setting in `--settings` | `.claude/settings.local.json` in the folder | login | server started | init entry | the tool call | the user's own servers in init |")
w("|---|---|---|---|---|---|---|---|---|")
for v, n in zip(V8, N8):
    m, argv = meta(n, "server_starts")[0], fx(n)[0]["_argv"]
    loaded = v in ("enable-all-false", "enabled-other")
    check(argv[argv.index("--setting-sources") + 1] == SET8[v][0] and "--strict-mcp-config" not in argv, "the setting sources of " + n)
    check((m["server_starts"] > 0) == loaded and bool(m["init_entry_for_this_server"]) == loaded and m["tool_in_init"] == loaded, "is the server loaded in " + n)
    local = meta(n, "settings_local_json_of_the_work_folder")
    check(len(local) == (n in FILE8), "the local settings file of " + n)
    if local:
        on = local[0]["settings_local_json_of_the_work_folder"]
        check(on["enableAllProjectMcpServers"] is True and on["enabledMcpjsonServers"] == ["ping-project"] and sorted(on["hooks"]) == ["SessionStart", "UserPromptSubmit"],
              "what the local settings file of %s turns on" % n)
    # Hooks: none in a logged-in run (`disableAllHooks`) and none where no file has one. The two hooks of the local
    # file ran in the runs without a login: the CLI did read that file.
    hooks = 2 if n in FILE8 and SET8[v][2] != "real" else 0
    check(m["hook_lines_in_the_stream"] == hooks and m["hook_file_lines"] == hooks, "the hooks that ran in " + n)
    check(m["servers_of_the_users_own_setup_in_init"] == (SET8[v][2] == "real"), "the user's own servers in " + n)
    check(m["result"] == ("pong" if loaded else "none" if SET8[v][2] == "real" else "Not logged in · Please run /login"), "the result of " + n)
    check(m["can_use_tool_requests"] == ([["mcp__ping-project__ping", {"name": "ping-project", "source": "project"}, "allowed"]] if loaded else []), "the requests of " + n)
    call = "called: `pong`" if loaded else "the model answered `none`" if SET8[v][2] == "real" else "not possible (no login)"
    w("| `%s` | `%s` | %s | %s | %s | %s | %s | %s | %s |" % (v, SET8[v][0], SET8[v][1], "turns the project's servers on" if local else "none", SET8[v][2],
                                                           "yes" if m["server_starts"] else "no",
                                                           "`%s`" % j(m["init_entry_for_this_server"][0]) if m["init_entry_for_this_server"] else "none", call,
                                                           "yes" if m["servers_of_the_users_own_setup_in_init"] else "no"))
w("")
h_on, h_off = meta("r6-project-mcp-hooks-on", "server_starts")[0], meta("r6-project-mcp-hooks-off", "server_starts")[0]
check(h_on["hook_file_lines"] == 2 and h_on["hook_lines_in_the_stream"] == 2 and h_off["hook_file_lines"] == 0 and h_off["hook_lines_in_the_stream"] == 0,
      "disableAllHooks in --settings stops the hooks of the user's settings")
w("""What the adapter can rely on (n=1 per row; the two `user-local-file-home` rows are the same scenario twice):
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
""")

# ================================================================= 9
w("## 9. `--resume` from another folder, and of a damaged transcript\n")
w("Question: the `initialize` answer, the last lines and the exit code, 3 runs each.\n")
N9O = ["r6-resume-other-folder-%d" % i for i in (1, 2, 3)]
N9T = ["r6-resume-damaged-tail-%d" % i for i in (1, 2, 3)]
N9M = ["r6-resume-damaged-mid-%d" % i for i in (1, 2, 3)]
w("Fixtures: `r6-resume-source` (the session that is resumed: two turns, the answers are `apple` and `pear`), `r6-resume-other-folder-1` … `-3` "
  "(`--resume <its id>` started in another work folder, a separate git repo next to the first), `r6-resume-damaged-tail-1` … `-3` (`--resume` of a copy of its transcript "
  "file whose LAST line is cut in half), `r6-resume-damaged-mid-1` … `-3` (a copy that ends in the middle of the line that holds the second message). "
  "Flags of a resume: %s. The host writes `initialize` right after the spawn. A process that answers it and stays gets one message.\n" % flags(N9O[0]))
w("The first run from another folder (the body of the `initialize` answer is left out: it is the usual long one):\n")
SEQ9 = lambda l: kind(l) in ("sent", "meta", "result", "stderr", "control_response") or sub(l) == "init" or has_text(l)


def no_body(line):
    """A quoted `initialize` answer without its body (`"…":"…"` stands for the keys left out, as everywhere)."""
    if not line.startswith('{"type":"control_response"'):
        return line
    r = json.loads(line)["response"]
    return j({"type": "control_response", "response": {"subtype": r["subtype"], "request_id": r["request_id"], "response": {"…": "…"}}})


w(block([no_body(x) for x in tl(N9O[0], SEQ9, text=80, maxlist=9)]))
w("| run | what was resumed | `initialize` answered | lines | the turn's answer | exit code | after the run |")
w("|---|---|---|---|---|---|---|")
for n in N9O + N9T + N9M:
    about = note(n, "about_the_session")
    check(note(n, "initialize_answered") and not note(n, "exited_by_itself") and note(n, "a_turn_was_sent") and note(n, "result_session_id_is_the_resumed_one"), "the start of " + n)
    check(note(n, "stdout_lines_in_order") == ["initialize answer (success)", "system/init", "result/success"] and exit_code(n) == 0 and not [l for l in fx(n) if "_stderr" in l],
          "the lines of " + n)
    answer = note(n, "turn_result")
    if n in N9O:
        files = note(n, "session_files_with_this_id")
        check(list(files) == ["-tmp-work-r6-resume-source"], "the session file stays in the first folder: " + n)
        check([l["cwd"] for l in fx(n) if sub(l) == "init"] == ["/tmp/work/r6-resume-elsewhere"], "the resumed process works in the new folder: " + n)
        what, after = "the whole session, from another folder", "the session's one file is still in the FIRST folder's project folder (%d lines)" % files["-tmp-work-r6-resume-source"]
    else:
        copy = note(n, "the_copy_after_the_run")
        check(copy["lines_that_are_not_json"] == 1, "the damaged line stays in " + n)
        what = "a copy cut in line %d of %d (a `%s` line; %d of its %d bytes kept); whole before the cut: %d message lines of the user, %d of the assistant" % (
            about["whole_lines_kept"] + 1, about["lines_of_the_source"], about["kind_of_the_cut_line"], about["bytes_kept_of_the_cut_line"], about["bytes_of_the_cut_line"],
            about["whole_user_lines_kept"], about["whole_assistant_lines_kept"])
        after = "the file has %d lines, 1 of them not JSON" % copy["lines"]
    check(sorted(answer.split()) == (["apple"] if n in N9M else ["apple", "pear"]), "what the resumed conversation holds: " + n)
    w("| `%s` | %s | yes | `%s` | `%s` | 0 | %s |" % (n[len("r6-resume-"):], what, "`, `".join(note(n, "stdout_lines_in_order")), answer.replace("\n", "\\n"), after))
w("")
check(all(note(n, "about_the_session")["kind_of_the_cut_line"] == "user" and note(n, "about_the_session")["whole_assistant_lines_kept"] == 1 for n in N9M), "where the mid copies are cut")
check(all(note(n, "about_the_session")["whole_assistant_lines_kept"] == 2 for n in N9T), "where the tail copies are cut")
C9 = [results(n)[-1]["total_cost_usd"] for n in ["r6-resume-source"] + N9O + N9T]
check(C9[1] > C9[0] and all(c < C9[0] for c in C9[4:]), "the session total goes on from another folder and starts again after a cut cost line")
check(all(note(n, "about_the_session")["kind_of_the_cut_line"] == "cost-state" for n in N9T), "the cut line of the tail copies")
w("""What the adapter can rely on (n=3 per row, the three runs of each row agree):
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
  on from the session's total (%.6f after %.6f). With the last line cut (a `cost-state` line) it started again
  (%.6f): a host that reads the session total after a resume cannot trust it across a damaged file.
- So for the two cases asked about there is no failure to tell apart from `SessionGone`: the start succeeds. The only
  recorded answer of the CLI to a conversation it cannot open is still the one of part 3 (unknown id).
""" % (C9[1], C9[0], C9[4]))

# ================================================================= 10
w("## 10. `--max-budget-usd` on a resumed process, reached in the middle of a turn\n")
w("Question: the limit is reached after a tool call of a turn on a RESUMED process: the result line, `total_cost_usd`, and a second turn after it.\n")
N10 = {label: sorted(n for n in ALL if n.startswith("r6-budget-midturn-%s-" % label)) for label in "12"}
C10 = [n for label in "12" for n in N10[label] if "-c" in n[-3:]]
w("Fixtures: `r6-budget-midturn-1-a`, `-1-b`, `-1-c1` … `-1-c3` and `r6-budget-midturn-2-a`, `-2-b`, `-2-c1` (the scenario twice). Run a: a new session, two turns. "
  "Run b: `--resume`, no budget, one turn (T = its cost). Runs c: `--resume` with `--max-budget-usd` = a part of T, one turn, then one more message. "
  "Flags of a run c: %s. Every turn asks for three tool calls in a row (`echo one`, `echo two`, `echo three`) and a one-word answer.\n" % flags(C10[0]))
w("The driver's note of every run:\n")
w(block([metas(n, "run", maxlist=9)[0] for label in "12" for n in N10[label]]))
GOOD10 = [n for n in C10 if note(n, "tool_calls_that_ran_in_the_last_turn") >= 1]
check(len(C10) == 4 and len(GOOD10) == 2, "two of the four budget runs reached the limit after a tool call had run")
w("`%s`, the run whose limit came after a tool call had run: every line but the stream events:\n" % GOOD10[0])
w(block(tl(GOOD10[0], lambda l: kind(l) in ("sent", "result", "assistant") or sub(l) == "init" or is_tool_result(l) or (kind(l) == "meta" and closing(l)),
           text=80, maxlist=9, keep=("errors",))))


def res10(n, i):
    r = results(n)[i]
    return j({k: r.get(k) for k in ("type", "subtype", "is_error", "num_turns", "stop_reason", "terminal_reason", "errors", "total_cost_usd", "result_index")}
             | {"model_usage_since_last_result": {k: {"costUSD": v.get("costUSD")} for k, v in (r.get("model_usage_since_last_result") or {}).items()}, "…": "…"},
             keep=("model_usage_since_last_result", "errors"))


w("Its two `result` lines (the turn that reached the limit, and the turn after it):\n")
w(block([res10(GOOD10[0], 0), res10(GOOD10[0], 1)]))
TEXT10 = "r6-budget-midturn-1-c1"  # the model answered in text and asked for no tool
w("`%s`: the model's answer and the `result` (the first try of the first recording):\n" % TEXT10)
w(block(tl(TEXT10, lambda l: has_text(l) or kind(l) == "result", stop=lambda l: kind(l) == "result", text=260, keep=("errors",))))
for n in C10:
    m = meta(n, "run")[0]
    r = results(n)
    check(len(r) == 2 and all(x["subtype"] == "error_max_budget_usd" and x["terminal_reason"] == "budget_exhausted" for x in r), "both results of " + n)
    check(m["second_turn"]["assistant_lines"] == 0 and m["second_turn"]["alive_after_it"] and m["alive_2s_after_the_limit"] and r[1]["total_cost_usd"] == r[0]["total_cost_usd"]
          and r[1]["num_turns"] == 1 and not r[1].get("model_usage_since_last_result"), "the second turn of " + n)
    uses = sum(1 for l in fx(n) if has_tool_use(l))
    if n == TEXT10:
        check(uses == 0 and r[0]["stop_reason"] == "end_turn" and "budget" in r[0].get("result", "") + json.dumps([l for l in fx(n) if has_text(l)]),
              "the run in which the model asked for no tool: " + n)
    else:
        check(uses == m["tool_calls_that_ran_in_the_last_turn"] + 1 and r[0]["stop_reason"] == "tool_use", "the limit comes right after a tool_use line in " + n)
    check(m["cost_of_the_last_turn_alone"] > m["budget_usd"] and r[0]["total_cost_usd"] > 5 * m["budget_usd"] and exit_code(n) == 1, "the numbers of " + n)
    check("--resume" in fx(n)[0]["_argv"], "a resumed process: " + n)
g10 = meta(GOOD10[0], "run")[0]
FIRST10 = [meta(n, "run")[0]["cost_of_the_last_turn_alone"] for n in C10 if n not in GOOD10]
T10 = meta("r6-budget-midturn-1-b", "run")[0]["cost_of_the_last_turn_alone"]
check(len(FIRST10) == 2 and all(x > 0.5 * T10 for x in FIRST10), "the first request of a resumed process costs more than half of the unlimited turn")
check(all(len({l["message"]["id"] for l in fx(n) if kind(l) == "assistant"}) == 1 for n in C10 if n not in GOOD10), "those two runs made one model request")
w("""What the adapter can rely on (n=4 processes that reached the limit: 2 after one tool call had run, 2 before any):
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
  %.6f USD against a budget of %.6f USD when it stopped (the request that crosses the limit is paid for).
- `total_cost_usd` of that `result` is the SESSION's total (%.6f USD: the earlier processes of the session are in
  it), far above the budget. `model_usage_since_last_result` is the turn alone.
- The process stays alive after the limit. A second message gets `system/init`, then at once the same error `result`
  (`num_turns` 1, `stop_reason` null, the same `total_cost_usd`, no `model_usage_since_last_result`): no model request
  is made (no `assistant` line). After stdin close the exit code is 1.
- So a process that reached its budget is of no more use: every later turn fails at once and costs nothing. To go
  on, the host needs a new process with a new budget.
- How coarse the limit is: the two runs that stopped after their FIRST model request had spent %s USD on it,
  while the whole turn of run b (four requests, no budget) cost %.6f USD. The first request of a resumed process is
  the dear one, so a budget below its cost stops the turn before any tool call.
""" % (g10["cost_of_the_last_turn_alone"], g10["budget_usd"], g10["total_cost_usd"], " and ".join("%.6f" % x for x in FIRST10), T10))

# ================================================================= 11
w("## 11. The same server name in `--mcp-config` and in the user's own setup\n")
w("Question: which one is loaded, and what do `init` and `mcp_status` say about `source`?\n")
N11 = ["r6-mcp-user-clash-1", "r6-mcp-user-clash-2", "r6-mcp-user-strict-1"]
w("Fixtures: `r6-mcp-user-clash-1`, `-2` (%s) and `r6-mcp-user-strict-1` (control: the same with `--strict-mcp-config`). A THROW-AWAY HOME whose `.claude.json` "
  "defines `ping-same` (it would answer `pong from user` and write `started-user.txt`); `--mcp-config` defines `ping-same` too (`pong from config`, "
  "`started-config.txt`). No login, so NO TOOL CALL is possible: the recordings show what is loaded.\n" % flags(N11[0]))
w("First recording: `mcp_status` with its answer, the init line, the result, the driver's note:\n")
w(block(tl(N11[0], lambda l: sub(l) == "init" or kind(l) in ("sent", "control_response", "meta", "result") and not (kind(l) == "sent" and l["_sent"].get("type") == "user"),
           maxlist=9, text=80)))
for n in N11:
    m = meta(n, "server_starts")[0]
    check(m["server_starts"] == {"started-config.txt": 1} and m["init_entries_for_this_name"] == [{"name": "ping-same", "status": "connected", "source": "dynamic"}]
          and m["other_servers_in_init"] == 0, "the --mcp-config definition is the one that runs in " + n)
    check(m["mcp_status_entries_for_this_name"] == [{"name": "ping-same", "status": "connected", "scope": "dynamic", "source": "dynamic"}], "mcp_status of " + n)
    check(m["result"] == "Not logged in · Please run /login", "no login in " + n)
w("""What the adapter can rely on (n=2, and the control with `--strict-mcp-config` is the same):
- The `--mcp-config` definition wins: only `started-config.txt` exists (one start), the user's definition was not
  started at all.
- There is ONE entry of that name. `init.mcp_servers` says `{"name":"ping-same","status":"connected","source":"dynamic"}`
  and `mcp_status` says `"scope":"dynamic"`, `"source":"dynamic"`, with the config of the `--mcp-config` definition.
  Nothing in the stream says that a user definition of the same name exists or was shadowed.
- The same as the clash with a project `.mcp.json` (part 3, section 11): a server that the host passes with
  `--mcp-config` cannot be replaced by a definition of the same name from the user's setup or from the repo.
- Not recorded: the `can_use_tool` request for it (no login in a throw-away HOME). In every recorded request the
  `mcp_server.source` is the init entry's `source`, so `dynamic` is the expected word, not a recorded one.
""")

# ================================================================= code
w("""## Can the waiting code be built now? (as of the day of recording)

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
""")

# ================================================================= table
w("""## Table: question -> answer -> fixture

| # | question | answer | n | fixture |
|---|---|---|---|---|
| 1 | An interrupt while a helper works | (a) foreground: the answer, the `Agent` call rejected, `task_updated` `killed`, `task_notification` `stopped`, the turn's error `result`; no list line. (b) background: it does NOT go on: the same two task lines and a list line `[]` come before the answer, with or without a turn running; no turn starts by itself. A foreground Bash call that had become a task gets a `task_notification` of its own (`stopped`, empty `output_file`, no `task_updated`) | 2 + 4 + 4 (with section 2) | `r6-interrupt-helper-foreground-1`, `-2`, `-background-1`, `-2`, `-idle-1`, `-2` |
| 2 | A helper's stop inside a shutdown; `stop_task` on a foreground helper | `interrupt` ends the helper; `stop_task` after it is a no-op; exit %.2f to %.2f s after stdin close, nothing left. `stop_task` without an interrupt starts a turn that runs to its end even after stdin close (exit %.2f / %.2f s later). Foreground helper: `killed` / `stopped`, the `Agent` call returns an error result, the turn goes on | 2 per variant | `r6-shutdown-helper-sequence-1`, `-2`, `-idle-interrupt-*`, `-idle-close-now-*`, `-idle-interrupt-again-*`, `-idle-outwait-*`, `-foreground-stop-*` |
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
  (through its builder). Measured here with the time stamp of each line: %s s from the interrupt to the `result`.
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
""" % (min(e[1] for v in ("sequence", "idle-interrupt") for e in E2[v]), max(e[1] for v in ("sequence", "idle-interrupt") for e in E2[v]),
       E2["idle-close-now"][0][1], E2["idle-close-now"][1][1], secs([t["result"] for t in T2SEQ])))

out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(OUT, "NOTES-part4.md")
with open(out, "w") as f:
    f.write("\n".join(P))
print("wrote", out, sum(len(x) for x in P), "characters")
