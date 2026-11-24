"""B3: a turn that FAILS by itself while a background helper works: does the helper go on, what does the list say,
what ends it?

The process runs with `--max-turns 2`. Turn 1 starts a helper in the background (its Bash call:
`echo begin; sleep 41; echo end`) and ends (two model requests: within the limit). Turn 2 asks for three Bash calls
one after the other, so its third model request passes the limit and the turn ends with an error `result`. The host
notes the list and the helper's `sleep` right after that result, then waits (70 s at most) for the helper's own
`task_notification` and (40 s at most) for a turn that the CLI starts by itself. Nothing is killed unless the run
leaves the `sleep` behind.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys, time
from drv import Session
from r5_lib import Procs, open_tasks, results, task_started
from r6_lib import is_result, kinds, system, tool_call, wait_alive
run = sys.argv[1] if len(sys.argv) > 1 else "1"
helper = Procs("sleep 41")
s = Session("r7-failed-turn-helper-%s" % run, extra=["--tools", "Bash,Agent", "--max-turns", "2"], hard_timeout=240)
s.user("Use the Agent tool with run_in_background set to true to have a subagent run the Bash command "
       "`echo begin; sleep 41; echo end` and report its output. Do not wait for the subagent: right after starting it, "
       "reply with the single word: started")
t = task_started(s, task_type="local_agent")
tid = t and t["task_id"]
print("helper task:", tid, "result 1:", s.wait_results(1, 60))
print("the helper's Bash call began:", bool(tool_call(s, "Bash", helper=True)), "its sleep runs:", wait_alive(helper))
m, n = s.mark(), results(s)
s.user("Run the Bash command `echo one`. After you see its output, run `echo two` in a second, separate tool call. After that, "
       "run `echo three` in a third, separate tool call. Then reply with the single word: done")
print("turn 2's result:", s.wait_results(n + 1, 60))
r2 = s.wait_for(is_result, 1, m) or {}
sleep_41_alive = bool(helper.alive())
with s.cv:
    s.note(right_after_the_failed_turn=dict(result_subtype=r2.get("subtype"), terminal_reason=r2.get("terminal_reason"), num_turns=r2.get("num_turns"),
                                            list=open_tasks(s), sleep_41_alive=sleep_41_alive, lines=kinds(s, m)))
m2, n2 = s.mark(), results(s)
end = s.wait_for(system("task_notification", task_id=tid), 70, m)
self_started = s.wait_results(n2 + 1, 40) if end else False
time.sleep(1.0)
sleep_41_alive = bool(helper.alive())
with s.cv:
    later = [e for e in s.events[m2:] if e.get("type") == "result"]
    s.note(after_it=dict(helper_end=end and end.get("status"), helper_summary=end and end.get("summary", "")[:120],
                         turn_by_itself=self_started, its_results=[[e.get("subtype"), e.get("origin")] for e in later],
                         lines=kinds(s, m2), list=open_tasks(s), sleep_41_alive=sleep_41_alive))
print("helper's end:", end and end.get("status"), "turn by itself:", self_started, "lines:", kinds(s, m2))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
print("killed leftovers:", helper.kill_left())
