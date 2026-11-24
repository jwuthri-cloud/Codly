"""A1: an `interrupt` while a helper (subagent) works.

argv[1]: foreground  the turn's `Agent` call still waits for the helper (run_in_background false). The helper's Bash
                     call runs `echo begin; sleep 127; echo end`; 3 s into it the host sends `interrupt` and reads for
                     10 s. Then `stop_task` with the helper's task id (is the id still known?) and one more turn.
         background  turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 43; echo end`) and
                     ends. Turn 2 runs `echo go; sleep 59` in the foreground; 3 s into it the host sends `interrupt`
                     and reads for 10 s. Then one more turn.
         idle        turn 1 starts the same background helper and ends. No turn runs: 3 s into the helper's Bash call
                     the host sends `interrupt` and reads for 10 s. If the helper still works after that, the script
                     waits for it to end by itself and for the turn the CLI starts after it. Then one more turn.
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
"N s into a call" counts from the moment the driver sees the call's `sleep` process, not from the `tool_use` line.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, next_turn, open_tasks, results, task_started
from r6_lib import answer_to, is_init, is_result, kinds, sent_at, stamps, system, tool_call, wait_alive
variant = sys.argv[1] if len(sys.argv) > 1 else "foreground"
run = sys.argv[2] if len(sys.argv) > 2 else "1"
s = Session("r6-interrupt-helper-%s-%s" % (variant, run), extra=["--tools", "Bash,Agent"], hard_timeout=220)


def since_interrupt():
    """Seconds from the interrupt request to the lines that follow it, by the stream's own time stamps."""
    return stamps(s, sent_at(s, "interrupt"), answer=answer_to(s), task_updated=system("task_updated"),
                  task_notification=system("task_notification"), result=is_result, next_init=is_init)


if variant == "foreground":
    sleeps = Procs("sleep 127")
    s.user("Use the Agent tool with run_in_background set to false to have a subagent run the Bash command "
           "`echo begin; sleep 127; echo end` and report its output. Then tell me what it reported.")
    t = task_started(s, task_type="local_agent")
    tid = t and t["task_id"]
    print("helper task:", tid, "is_backgrounded:", t and t.get("is_backgrounded"))
    print("the helper's Bash call began:", bool(tool_call(s, "Bash", helper=True)), "its sleep runs:", wait_alive(sleeps))
    time.sleep(3.0)
    s.note(before_the_interrupt=dict(list=open_tasks(s), helper_is_backgrounded=t and t.get("is_backgrounded"), sleep_127_alive=bool(sleeps.alive())))
    m, n = s.mark(), results(s)
    ask(s, "interrupt")
    got = s.wait_results(n + 1, 30)
    time.sleep(10.0)
    s.note(after_the_interrupt=dict(result_came=got, lines=kinds(s, m), results_in_the_10s_after=results(s) - n, sleep_127_alive=bool(sleeps.alive()),
                                    list=open_tasks(s)), seconds_from_the_interrupt_request=since_interrupt())
    print("lines after the interrupt:", kinds(s, m))
    ask(s, "stop_task", task_id=tid)
    print("next turn:", next_turn(s, "Reply with the single word: alive", 40))
    time.sleep(2.0)
    s.note(at_the_end=dict(sleep_127_alive=bool(sleeps.alive()), list=open_tasks(s), results=results(s)))
    print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
    print("killed leftovers:", sleeps.kill_left())
    sys.exit(0)

helper, fg = Procs("sleep 43"), Procs("sleep 59")
s.user("Use the Agent tool with run_in_background set to true to have a subagent run the Bash command "
       "`echo begin; sleep 43; echo end` and report its output. Do not wait for the subagent: right after starting it, "
       "reply with the single word: started")
t = task_started(s, task_type="local_agent")
tid = t and t["task_id"]
print("helper task:", tid, "is_backgrounded:", t and t.get("is_backgrounded"), "result 1:", s.wait_results(1, 60))
print("the helper's Bash call began:", bool(tool_call(s, "Bash", helper=True)), "its sleep runs:", wait_alive(helper))
if variant == "background":
    m = s.mark()
    s.user("Run the Bash command `echo go; sleep 59` and then reply with the single word: done")
    print("the turn's own Bash call began:", bool(tool_call(s, "Bash", helper=False, start=m)), "its sleep runs:", wait_alive(fg))
time.sleep(3.0)
s.note(before_the_interrupt=dict(list=open_tasks(s), a_turn_runs=variant == "background", sleep_43_alive=bool(helper.alive()), sleep_59_alive=bool(fg.alive())))
m, n = s.mark(), results(s)
ask(s, "interrupt")
time.sleep(10.0)
s.note(after_the_interrupt=dict(lines=kinds(s, m), results_in_the_10s_after=results(s) - n, sleep_43_alive=bool(helper.alive()), sleep_59_alive=bool(fg.alive()),
                                list=open_tasks(s)), seconds_from_the_interrupt_request=since_interrupt())
print("lines after the interrupt:", kinds(s, m), "helper's sleep alive:", bool(helper.alive()))
if helper.alive():
    m, n = s.mark(), results(s)
    end = s.wait_for(system("task_notification", task_id=tid), 70, m)
    print("the helper's end:", end and end.get("status"), "turn by itself:", s.wait_results(n + 1, 40))
    time.sleep(1.0)
    s.note(the_helper_ended_by_itself=dict(status=end and end.get("status"), lines=kinds(s, m), list=open_tasks(s), sleep_43_alive=bool(helper.alive())))
print("next turn:", next_turn(s, "Reply with the single word: alive", 40))
time.sleep(2.0)
s.note(at_the_end=dict(sleep_43_alive=bool(helper.alive()), list=open_tasks(s), results=results(s)))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
print("killed leftovers:", helper.kill_left(), fg.kill_left())
