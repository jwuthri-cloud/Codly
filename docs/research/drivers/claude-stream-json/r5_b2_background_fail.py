"""B2: a background command that fails by itself, and whether a FOREGROUND helper or command is ever in the list.

Turn 1 starts `sleep 4; exit 1` in the background and ends; the CLI then starts a turn by itself.
Turn 3 runs a subagent in the foreground (`echo a; sleep 6; echo b`).
Turn 4 runs a foreground Bash call (`echo a; sleep 20; echo b`); 8 s into it the host sends `background_tasks`
(per the bundle it moves running foreground tasks to the background) and reads what follows.
"""
import time
from drv import Session
from r5_lib import ask, open_tasks, results, task_started
s = Session("r5-background-fail", extra=["--tools", "Bash,Agent"], hard_timeout=240)

def lists(start):
    with s.cv:
        return [e["tasks"] for e in s.events[start:] if e.get("type") == "system" and e.get("subtype") == "background_tasks_changed"]

s.user("Run `sleep 4; exit 1` with the Bash tool using run_in_background: true. Do not wait for it and do not check its "
       "output: right after starting it, reply with the single word: started")
print("result 1:", s.wait_results(1, 60))
print("result 2 (self-started):", s.wait_results(2, 40))
time.sleep(1.0)

m = s.mark()
s.user("Use the Agent tool with run_in_background set to false to have a subagent run the Bash command "
       "`echo a; sleep 6; echo b` and report its output. Then tell me what it reported.")
t = task_started(s, m, task_type="local_agent")
print("subagent task is_backgrounded:", t and t.get("is_backgrounded"), "result 3:", s.wait_results(3, 90))
s.note(turn="foreground subagent", is_backgrounded=t and t.get("is_backgrounded"), background_tasks_changed_lines=lists(m))

m = s.mark()
s.user("Run the Bash command `echo a; sleep 20; echo b` (in the foreground, do not use run_in_background) and then "
       "reply with the single word: done")
print("tool started:", bool(s.wait_tool_use(m, name="Bash")))
time.sleep(8.0)
s.note(lists_before_background_tasks_request=lists(m))
ask(s, "background_tasks")
n = results(s)
print("result 4:", s.wait_results(n + 1, 60))
time.sleep(1.0)
s.note(lists_after_background_tasks_request=lists(m), list_now=open_tasks(s))
print("result 5 (self-started, if the command was moved):", s.wait_results(n + 2, 40))
time.sleep(1.0)
s.note(lists_of_turn_4_and_after=lists(m), results_seen=results(s))
print("results:", results(s), "cost:", s.cost())
print("exit:", s.close(wait=30))
