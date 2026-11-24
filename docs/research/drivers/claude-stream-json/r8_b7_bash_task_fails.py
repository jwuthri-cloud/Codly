"""B7: a foreground Bash call that becomes a task (it runs longer than about 3 s: part 5, section 12) and FAILS (exit
code 3), and one that is cut by an `interrupt`: the status word and the keys of their `task_notification`.

One process, two turns. Turn 1: `echo q7; sleep 4; exit 3` in the foreground. Turn 2: `echo q7b; sleep 37` in the
foreground; 1 s after the driver sees its `task_started` line and its `sleep`, the host sends `interrupt`. After each
turn a note: the call's `task_started`, every task line of the turn (whole), the tool result (cut), the result's
subtype; for turn 2 the `sleep` after the interrupt.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, next_turn, results, task_started
from r6_lib import kinds, wait_alive
from r7_lib import tool_results
from r8_lib import last_result
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
cut = Procs("sleep 37")
s = Session("r8-bash-task-fails-%s" % LABEL, extra=["--tools", "Bash"], hard_timeout=150)


def task_lines(start):
    with s.cv:
        return [{k: v for k, v in e.items() if k not in ("uuid", "session_id")} for e in s.events[start:]
                if e.get("type") == "system" and e.get("subtype") in ("task_started", "task_updated", "task_notification", "background_tasks_changed")]


m = s.mark()
ok = next_turn(s, "Run the Bash command `echo q7; sleep 4; exit 3` once, in the foreground, and then reply with the single word: done", 60)
s.note(turn="fails", ok=ok, task_lines=task_lines(m), tool_results=tool_results(s, m), result_subtype=last_result(s, m).get("subtype"))
m, n = s.mark(), results(s)
s.user("Run the Bash command `echo q7b; sleep 37` once, in the foreground, and then reply with the single word: done")
started = task_started(s, m, 30, task_type="local_bash")
print("task:", bool(started), "sleep runs:", wait_alive(cut, 20))
time.sleep(1.0)
m2 = s.mark()
ask(s, "interrupt")
print("result:", s.wait_results(n + 1, 30))
time.sleep(1.0)
alive = bool(cut.alive())
with s.cv:
    s.note(turn="interrupted", task_lines=task_lines(m), lines_since_the_interrupt=kinds(s, m2), tool_results=tool_results(s, m),
           result_subtype=last_result(s, m).get("subtype"), sleep_37_alive=alive)
print("lines:", kinds(s, m2))
print("cost:", s.cost(), "exit:", s.close())
print("killed leftovers:", cut.kill_left())
