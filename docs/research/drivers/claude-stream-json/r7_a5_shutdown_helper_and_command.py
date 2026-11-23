"""A5: a shutdown with a background helper AND a background command at once: `interrupt`, `stop_task` for the
command, stdin closed. The order of the lines, the exit time, anything left running?

Turn 1 starts a background command (`echo bg; sleep 197`, run_in_background true) and a background helper (its Bash
call: `echo begin; sleep 199; echo end`), and ends. The host waits until it sees both `sleep` processes, and 2 s more.
Then: `interrupt` -> its answer -> `stop_task` with the command's task id -> its answer -> stdin closed.
The CLI is started as the leader of its own process group, as the app starts it. `process_exited` is noted at the
moment the process is gone (r6_lib.note_exit). After the exit the host notes the scenario's `sleep` processes and
every process still in the CLI's process group.

Each note that lists lines is written under the driver's lock in the same step as the list (no line can come between
the list and the note's time stamp); process checks are made BEFORE that step and noted with it.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import subprocess, sys, time
from drv import Session
from r5_lib import Procs, ask, open_tasks, results, task_started
from r6_lib import answer_to, kinds, note_exit, sent_at, stamps, system, wait_alive
run = sys.argv[1] if len(sys.argv) > 1 else "1"
cmd, helper = Procs("[s]leep 197"), Procs("[s]leep 199")
s = Session("r7-shutdown-helper-and-command-%s" % run, extra=["--tools", "Bash,Agent"], hard_timeout=200, own_group=True)
s.user("Do two things in one message, then reply with the single word: started. 1) Run the Bash command `echo bg; sleep 197` "
       "with run_in_background set to true. 2) Use the Agent tool with run_in_background set to true to have a subagent run the "
       "Bash command `echo begin; sleep 199; echo end` and report its output. Do not wait for either of them.")
print("result 1:", s.wait_results(1, 60))
command = task_started(s, task_type="local_bash", background=True, timeout=20)
helper_task = task_started(s, task_type="local_agent", timeout=20)
print("both sleeps run:", wait_alive(cmd, 30), wait_alive(helper, 30))
time.sleep(2.0)
alive = dict(sleep_197_alive=bool(cmd.alive()), sleep_199_alive=bool(helper.alive()))
s.note(before_the_stop=dict(list=open_tasks(s), a_turn_runs=results(s) == 0, command_task=command and command["task_id"],
                            helper_task=helper_task and helper_task["task_id"], **alive))
m0 = s.mark()
ask(s, "interrupt")
ask(s, "stop_task", task_id=command and command["task_id"])
stop_answer = answer_to(s)
time.sleep(0.5)
alive = dict(sleep_197_alive=bool(cmd.alive()), sleep_199_alive=bool(helper.alive()))
with s.cv:
    s.note(before_stdin_is_closed=dict(lines_since_the_interrupt=kinds(s, m0), list=open_tasks(s), **alive),
           seconds_from_the_interrupt=stamps(s, sent_at(s, "interrupt"), answer=lambda e: e.get("type") == "control_response" and e["response"].get("request_id") == "host-1",
                                             helper_notification=system("task_notification", task_id=helper_task and helper_task["task_id"]),
                                             command_notification=system("task_notification", task_id=command and command["task_id"])),
           seconds_from_stop_task=stamps(s, sent_at(s, "stop_task"), answer=stop_answer,
                                         command_notification=system("task_notification", task_id=command and command["task_id"])))
note_exit(s)
m = s.mark()


def group_left():
    """Processes still in the CLI's process group (the CLI was its leader: the group id is its pid)."""
    out = subprocess.run(["pgrep", "-g", str(s.p.pid)], capture_output=True, text=True).stdout.split()
    return [subprocess.run(["ps", "-o", "command=", "-p", p], capture_output=True, text=True).stdout.strip()[:80] for p in out]


def after_exit():
    time.sleep(1.0)
    alive = dict(sleep_197_alive=bool(cmd.alive()), sleep_199_alive=bool(helper.alive()), processes_left_in_the_group=group_left())
    with s.cv:
        return dict(after_the_exit=dict(lines_after_stdin_was_closed=kinds(s, m), **alive))


code = s.close(wait=60, after_exit=after_exit)
print("lines since the interrupt:", kinds(s, m0))
print("exit code:", code, "results:", results(s), "cost:", s.cost())
print("killed leftovers:", cmd.kill_left(), helper.kill_left())
