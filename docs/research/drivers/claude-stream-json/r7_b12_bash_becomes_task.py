"""B12: a foreground Bash call that becomes a task and ends by itself: its `task_notification` (status word,
`output_file`), and how long a call must run before it becomes a task.

One process, one turn per duration: "Run `echo q12; sleep D` and reply done", for D in DURATIONS. After each turn a
note: did a `task_started` (`local_bash`) come for the call, how many seconds after the call's `tool_use` line, and the
`task_notification` of that task (status, output_file, seconds after `task_started`), all from the time stamps of the
lines. The commands end by themselves; nothing is killed.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys
from drv import Session
from r5_lib import next_turn
from r6_lib import raw, system
from r7_lib import is_tool_result, is_tool_use, last_sent
run = sys.argv[1] if len(sys.argv) > 1 else "1"
DURATIONS = (1, 2, 3, 4, 6, 9, 14)
s = Session("r7-bash-becomes-task-%s" % run, extra=["--tools", "Bash"], hard_timeout=400)


def first(since, pred):
    for r in raw(s):
        if r["dir"] == "recv" and r["t"] >= since and isinstance(r["msg"], dict) and pred(r["msg"]):
            return r
    return None


for d in DURATIONS:
    cmd = "echo q12; sleep %d" % d
    ok = next_turn(s, "Run the Bash command `%s` (one call, in the foreground, not in the background) and then reply with the single word: done" % cmd, 90)
    since = last_sent(s)
    use = first(since, is_tool_use("Bash"))
    started = first(since, system("task_started", task_type="local_bash"))
    done = first(since, system("task_notification"))
    res = first(since, is_tool_result())
    rel = lambda r, base: round(r["t"] - base["t"], 3) if r and base else None
    s.note(duration=d, command_in_the_call=use and [b["input"].get("command") for b in use["msg"]["message"]["content"] if b.get("type") == "tool_use"],
           turn_ok=ok, task_started=bool(started),
           seconds_from_tool_use_to_task_started=rel(started, use), seconds_from_tool_use_to_tool_result=rel(res, use),
           task_notification=done and {k: done["msg"].get(k) for k in ("status", "output_file", "summary")},
           seconds_from_task_started_to_task_notification=rel(done, started),
           task_notification_before_tool_result=bool(done and res and done["t"] <= res["t"]))
    print("sleep %2d: task=%s tool_use->task_started=%s notification=%s" % (d, bool(started), rel(started, use), done and done["msg"].get("status")))
print("cost:", s.cost(), "exit:", s.close())
