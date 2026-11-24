"""B1: stop_task on a running background SUBAGENT (task_type local_agent).

The subagent runs `echo begin; sleep 113; echo end` with Bash. The turn that started it ends; 3 s after the subagent's
Bash call began, the host sends stop_task with the subagent's task id and reads for 20 s. Then get_task_output,
stop_task again, and one more turn to see that the session still works.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, next_turn, open_tasks, results, task_started
run = sys.argv[1] if len(sys.argv) > 1 else "1"
sleeps = Procs("sleep 113")
s = Session("r5-stop-subagent-" + run, extra=["--tools", "Bash,Agent"], hard_timeout=200)

s.user("Use the Agent tool with run_in_background set to true to have a subagent run the Bash command "
       "`echo begin; sleep 113; echo end` and report its output. Do not wait for the subagent: right after starting it, "
       "reply with the single word: started")
t = task_started(s, task_type="local_agent")
tid = t and t["task_id"]
print("subagent task:", tid, "is_backgrounded:", t and t.get("is_backgrounded"))
print("result 1:", s.wait_results(1, 60))
inner = s.wait_for(lambda e: e.get("type") == "assistant" and e.get("parent_tool_use_id") and any(
    b.get("type") == "tool_use" and b.get("name") == "Bash" for b in e["message"]["content"]), 60)
print("the subagent's Bash call began:", bool(inner))
time.sleep(3.0)
s.note(list_before_stop=open_tasks(s), host_sees_sleep_113_before_stop=bool(sleeps.alive()))
ask(s, "get_task_output", task_id=tid)
m, n = s.mark(), results(s)
ask(s, "stop_task", task_id=tid)
got = s.wait_results(n + 1, 20)
with s.cv:
    after = [e for e in s.events[m:] if e.get("parent_tool_use_id") and e.get("type") in ("assistant", "user")]
s.note(result_within_20s_of_stop_task=got, host_sees_sleep_113_after_stop=bool(sleeps.alive()),
       subagent_lines_after_stop_task=len(after), list_after_stop=open_tasks(s))
print("turn by itself after the stop:", got, "subagent lines after the stop:", len(after), "sleep alive:", sleeps.alive())
ask(s, "get_task_output", task_id=tid)
ask(s, "stop_task", task_id=tid)
print("next turn:", next_turn(s, "Reply with the single word: alive", 40))
time.sleep(2.0)
s.note(host_sees_sleep_113_at_the_end=bool(sleeps.alive()), list_at_the_end=open_tasks(s))
print("results:", results(s), "cost:", s.cost())
print("exit:", s.close())
print("killed leftovers:", sleeps.kill_left())
