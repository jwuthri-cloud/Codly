"""A11: a shutdown with everything at once: a TURN running (its foreground Bash call), a background helper and a
background command. Part 5 (section 5) recorded the same with no turn running.

Turn 1 starts a background command (`echo bg; sleep 229`, run_in_background) and a background helper (its Bash call:
`echo begin; sleep 233; echo end`) and ends. Turn 2 runs `echo fg; sleep 239` in the foreground. 2 s after the driver
sees all three `sleep` processes the host sends `interrupt`, waits for turn 2's `result` (10 s at most), sends
`stop_task` with the command's task id, and closes stdin 0.5 s after its answer.
The CLI is the leader of its own process group, as the app starts it. `process_exited` is noted at the moment the
process is gone (r6_lib.note_exit). After the exit the host notes the scenario's `sleep` processes and every process
still in the CLI's process group. Notes that list lines are written under the driver's lock in the same step as the
list; process checks are made before that step.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import subprocess, sys, time
from drv import Session
from r5_lib import Procs, ask, open_tasks, results, task_started
from r6_lib import answer_to, is_result, kinds, note_exit, sent_at, stamps, system, tool_call, wait_alive
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
cmd, helper, fg = Procs("sleep 229"), Procs("sleep 233"), Procs("sleep 239")
s = Session("r8-shutdown-all-%s" % LABEL, extra=["--tools", "Bash,Agent"], hard_timeout=200, own_group=True)
s.user("Do two things in one message, then reply with the single word: started. 1) Run the Bash command `echo bg; sleep 229` "
       "with run_in_background set to true. 2) Use the Agent tool with run_in_background set to true to have a subagent run the "
       "Bash command `echo begin; sleep 233; echo end` and report its output. Do not wait for either of them.")
print("result 1:", s.wait_results(1, 60))
command = task_started(s, task_type="local_bash", background=True, timeout=20)
helper_task = task_started(s, task_type="local_agent", timeout=20)
print("both sleeps run:", wait_alive(cmd, 30), wait_alive(helper, 30))
m = s.mark()
s.user("Run the Bash command `echo fg; sleep 239` in the foreground and then reply with the single word: done")
print("the turn's Bash call began:", bool(tool_call(s, "Bash", helper=False, start=m)), "its sleep runs:", wait_alive(fg, 30))
time.sleep(2.0)
ids = dict(command_task=command and command["task_id"], helper_task=helper_task and helper_task["task_id"])
alive = dict(sleep_229_alive=bool(cmd.alive()), sleep_233_alive=bool(helper.alive()), sleep_239_alive=bool(fg.alive()))
with s.cv:
    s.note(before_the_stop=dict(list=open_tasks(s), a_turn_runs=results(s) == 1, **ids, **alive))
m0, n = s.mark(), results(s)
ask(s, "interrupt")
interrupt_answer = answer_to(s)
print("turn 2's result:", s.wait_results(n + 1, 10))
ask(s, "stop_task", task_id=ids["command_task"])
stop_answer = answer_to(s)
time.sleep(0.5)
alive = dict(sleep_229_alive=bool(cmd.alive()), sleep_233_alive=bool(helper.alive()), sleep_239_alive=bool(fg.alive()))
with s.cv:
    s.note(before_stdin_is_closed=dict(lines_since_the_interrupt=kinds(s, m0), list=open_tasks(s), **alive),
           seconds_from_the_interrupt=stamps(s, sent_at(s, "interrupt"), answer=interrupt_answer, result=is_result,
                                             helper_notification=system("task_notification", task_id=ids["helper_task"]),
                                             command_notification=system("task_notification", task_id=ids["command_task"])),
           seconds_from_stop_task=stamps(s, sent_at(s, "stop_task"), answer=stop_answer,
                                         command_notification=system("task_notification", task_id=ids["command_task"])))
note_exit(s)
m = s.mark()


def group_left():
    """Processes still in the CLI's process group (the CLI was its leader: the group id is its pid)."""
    out = subprocess.run(["pgrep", "-g", str(s.p.pid)], capture_output=True, text=True).stdout.split()
    return [subprocess.run(["ps", "-o", "command=", "-p", p], capture_output=True, text=True).stdout.strip()[:80] for p in out]


def after_exit():
    time.sleep(1.0)
    alive = dict(sleep_229_alive=bool(cmd.alive()), sleep_233_alive=bool(helper.alive()), sleep_239_alive=bool(fg.alive()),
                 processes_left_in_the_group=group_left())
    with s.cv:
        return dict(after_the_exit=dict(lines_after_stdin_was_closed=kinds(s, m), **alive))


code = s.close(wait=60, after_exit=after_exit)
print("lines since the interrupt:", kinds(s, m0))
print("exit code:", code, "results:", results(s), "cost:", s.cost())
print("killed leftovers:", cmd.kill_left(), helper.kill_left(), fg.kill_left())
