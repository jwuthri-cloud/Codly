"""A2: the stop of a helper inside a shutdown, as one run; and `stop_task` on a helper whose `Agent` call still waits.

All variants but the last share the start: turn 1 starts a helper in the background (its Bash call:
`echo begin; sleep 139; echo end`) and ends.

argv[1]: sequence              a turn runs (`echo go; sleep 67`). 2 s into it: `interrupt` -> (the stopped turn's
                               result) -> `stop_task` with the helper's task id -> stdin closed
         idle-interrupt        no turn runs. `interrupt` -> `stop_task` with the helper's task id -> stdin closed
         idle-close-now        no turn runs. `stop_task` with the helper's task id (the CLI then starts a turn by
                               itself: part 3, section 1) -> stdin closed right after the answer to `stop_task`
         idle-interrupt-again  no turn runs. `stop_task` -> the host waits for the `system/init` of the self-started
                               turn -> `interrupt` -> that turn's result -> stdin closed
         idle-outwait          no turn runs. `stop_task` -> the self-started turn's result -> stdin closed
         foreground-stop       another scenario: one turn whose `Agent` call waits for the helper (run_in_background
                               false); 3 s into the helper's Bash call the host sends `stop_task` with the helper's task
                               id and reads until the turn's result (30 s at most); then one more turn
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
The CLI is started as the leader of its own process group, as the app starts it. A note is written at the moment the
process is gone (`process_exited`), so "how long after stdin was closed" is the difference of two time stamps of the
recording. "N s into a call" counts from the moment the driver sees the call's `sleep` process.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, next_turn, open_tasks, results, task_started
from r6_lib import answer_to, is_init, is_result, kinds, note_exit, sent_at, stamps, system, tool_call, wait_alive
variant = sys.argv[1] if len(sys.argv) > 1 else "sequence"
run = sys.argv[2] if len(sys.argv) > 2 else "1"
helper, fg = Procs("sleep 139"), Procs("sleep 67")
s = Session("r6-shutdown-helper-%s-%s" % (variant, run), extra=["--tools", "Bash,Agent"], hard_timeout=220, own_group=True)

if variant == "foreground-stop":
    s.user("Use the Agent tool with run_in_background set to false to have a subagent run the Bash command "
           "`echo begin; sleep 139; echo end` and report its output. Then tell me what it reported.")
    t = task_started(s, task_type="local_agent")
    tid = t and t["task_id"]
    print("helper task:", tid, "is_backgrounded:", t and t.get("is_backgrounded"))
    print("the helper's Bash call began:", bool(tool_call(s, "Bash", helper=True)), "its sleep runs:", wait_alive(helper))
    time.sleep(3.0)
    s.note(before_stop_task=dict(list=open_tasks(s), helper_is_backgrounded=t and t.get("is_backgrounded"), sleep_139_alive=bool(helper.alive())))
    m, n = s.mark(), results(s)
    ask(s, "stop_task", task_id=tid)
    got = s.wait_results(n + 1, 30)
    time.sleep(3.0)
    s.note(after_stop_task=dict(turn_result_within_30s=got, lines=kinds(s, m), sleep_139_alive=bool(helper.alive()), list=open_tasks(s), results=results(s) - n),
           seconds_from_the_stop_task_request=stamps(s, sent_at(s, "stop_task"), answer=answer_to(s), task_updated=system("task_updated"),
                                                     task_notification=system("task_notification"), result=is_result))
    print("lines after stop_task:", kinds(s, m), "helper's sleep alive:", bool(helper.alive()))
    print("next turn:", next_turn(s, "Reply with the single word: alive", 40))
    print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
    print("killed leftovers:", helper.kill_left())
    sys.exit(0)

s.user("Use the Agent tool with run_in_background set to true to have a subagent run the Bash command "
       "`echo begin; sleep 139; echo end` and report its output. Do not wait for the subagent: right after starting it, "
       "reply with the single word: started")
t = task_started(s, task_type="local_agent")
tid = t and t["task_id"]
print("helper task:", tid, "result 1:", s.wait_results(1, 60))
print("the helper's Bash call began:", bool(tool_call(s, "Bash", helper=True)), "its sleep runs:", wait_alive(helper))
if variant == "sequence":
    m = s.mark()
    s.user("Run the Bash command `echo go; sleep 67` and then reply with the single word: done")
    print("the turn's own Bash call began:", bool(tool_call(s, "Bash", helper=False, start=m)), "its sleep runs:", wait_alive(fg))
time.sleep(2.0)
s.note(before_the_stop=dict(list=open_tasks(s), a_turn_runs=variant == "sequence", sleep_139_alive=bool(helper.alive()), sleep_67_alive=bool(fg.alive())))

m0, n = s.mark(), results(s)
times = {}
if variant in ("sequence", "idle-interrupt"):
    ask(s, "interrupt")
    if variant == "sequence":
        print("the stopped turn's result:", s.wait_results(n + 1, 30))
    times["seconds_from_the_interrupt"] = stamps(s, sent_at(s, "interrupt"), answer=answer_to(s), task_notification=system("task_notification"), result=is_result)
    s.note(after_the_interrupt=dict(list=open_tasks(s), sleep_139_alive=bool(helper.alive()), sleep_67_alive=bool(fg.alive())))
m, n = s.mark(), results(s)
ask(s, "stop_task", task_id=tid)
stop_answer = answer_to(s)
if variant == "idle-interrupt-again":
    print("the self-started turn began:", bool(s.wait_for(is_init, 15, m)))
    ask(s, "interrupt")
    print("its result:", s.wait_results(n + 1, 30))
    times["seconds_from_the_interrupt"] = stamps(s, sent_at(s, "interrupt"), answer=answer_to(s), result=is_result)
elif variant == "idle-outwait":
    print("the self-started turn's result:", s.wait_results(n + 1, 40))
alive = dict(sleep_139_alive=bool(helper.alive()), sleep_67_alive=bool(fg.alive()))
# The process checks come first: the lines, the times and the note are then taken under the driver's lock in one step,
# so no line can come between the list and the note's time stamp (the recordings of part 4 were made before this fix).
with s.cv:
    times["seconds_from_stop_task"] = stamps(s, sent_at(s, "stop_task"), answer=stop_answer, task_notification=system("task_notification"),
                                             init_of_a_turn=is_init, result_of_a_turn=is_result)
    s.note(before_stdin_is_closed=dict(lines_since_the_first_request=kinds(s, m0), list=open_tasks(s), **alive), **times)
note_exit(s)
m = s.mark()


def after_exit():
    time.sleep(1.0)
    return dict(after_the_exit=dict(lines_after_stdin_was_closed=kinds(s, m), sleep_139_alive=bool(helper.alive()), sleep_67_alive=bool(fg.alive())))


code = s.close(wait=60, after_exit=after_exit)
print("lines since the first request:", kinds(s, m0))
print("exit code:", code, "results:", results(s), "cost:", s.cost())
print("left: sleep 139", helper.alive(), "sleep 67", fg.alive())
print("killed leftovers:", helper.kill_left(), fg.kill_left())
