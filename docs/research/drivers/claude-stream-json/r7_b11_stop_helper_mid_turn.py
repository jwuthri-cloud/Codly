"""B11: `stop_task` on a background helper WHILE a turn of the main agent runs: does a turn start by itself after the
running one?

Turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 173; echo end`) and ends. Turn 2 runs
`echo go; sleep 21.5` in the foreground. 2 s into that call (counted from the moment the driver sees its `sleep`) the
host sends `stop_task` with the helper's task id. The host reads until turn 2's result and then 20 s more, and notes
every line since the request. Part 3 (section 1) and part 4 (section 2) sent the same request with NO turn running:
the CLI then started a turn by itself.
After the exit a note counts, from the session's own transcript, the task notifications that waited in the CLI's
queue and those handed to a model request of the running turn (r7_lib.queued_notifications; counts only).

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, open_tasks, results, task_started
from r6_lib import answer_to, is_init, is_result, kinds, sent_at, stamps, system, tool_call, wait_alive
from r7_lib import queued_notifications
run = sys.argv[1] if len(sys.argv) > 1 else "1"
helper, fg = Procs("[s]leep 173"), Procs("[s]leep 21.5")
s = Session("r7-stop-helper-mid-turn-%s" % run, extra=["--tools", "Bash,Agent"], hard_timeout=200)
s.user("Use the Agent tool with run_in_background set to true to have a subagent run the Bash command "
       "`echo begin; sleep 173; echo end` and report its output. Do not wait for the subagent: right after starting it, "
       "reply with the single word: started")
t = task_started(s, task_type="local_agent")
tid = t and t["task_id"]
print("helper task:", tid, "result 1:", s.wait_results(1, 60))
print("the helper's Bash call began:", bool(tool_call(s, "Bash", helper=True)), "its sleep runs:", wait_alive(helper))
m = s.mark()
s.user("Run the Bash command `echo go; sleep 21.5` and then reply with the single word: done")
print("the turn's own Bash call began:", bool(tool_call(s, "Bash", helper=False, start=m)), "its sleep runs:", wait_alive(fg))
time.sleep(2.0)
s.note(before_stop_task=dict(list=open_tasks(s), a_turn_runs=results(s) == 1, sleep_173_alive=bool(helper.alive()), sleep_21_5_alive=bool(fg.alive())))
m, n = s.mark(), results(s)
ask(s, "stop_task", task_id=tid)
stop_answer = answer_to(s)
print("turn 2's result:", s.wait_results(n + 1, 60))
time.sleep(20.0)
r = [e for e in s.events[m:] if e.get("type") == "result"]
s.note(after_stop_task=dict(lines=kinds(s, m), results_since_the_request=len(r), result_origins=[e.get("origin") for e in r],
                            list=open_tasks(s), sleep_173_alive=bool(helper.alive()), sleep_21_5_alive=bool(fg.alive())),
       seconds_from_stop_task=stamps(s, sent_at(s, "stop_task"), answer=stop_answer, task_notification=system("task_notification", task_id=tid),
                                     result=is_result, init_after_it=is_init))
print("lines since stop_task:", kinds(s, m))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close(after_exit=lambda: dict(transcript=queued_notifications(s.sid))))
print("killed leftovers:", helper.kill_left(), fg.kill_left())
