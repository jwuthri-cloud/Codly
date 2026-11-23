"""B6: the full life of a background Bash task, read with get_task_output, and stop_task on a real task id.

Turn 1 starts `sleep 8; echo woke` in the background and ends. The host reads the task with get_task_output while it
runs and after it ended (the CLI then starts a turn by itself). Turn 3 starts `sleep 127; echo late` in the background;
the host sends stop_task with its id and keeps reading for 15 s.
"""
import subprocess, time
from drv import Session
s = Session("r4-background-life", extra=["--tools", "Bash"], hard_timeout=200)

def task_id(start):
    ev = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "task_started" and e.get("is_backgrounded"), 60, start)
    return ev and ev["task_id"]

def alive(pattern):
    """Is a process whose command line holds `pattern` alive? (the pattern is written so that it cannot match itself)"""
    return [p for p in subprocess.run(["pgrep", "-f", pattern], capture_output=True, text=True).stdout.split() if p not in BEFORE]

BEFORE = ()
BEFORE = alive("[s]leep 127")  # processes of other programs that happen to match: never counted, never touched

s.user("Run `sleep 8; echo woke` with the Bash tool using run_in_background: true. Do not wait for it and do not check its output: right after starting it, reply with the single word: started")
t1 = task_id(0)
print("task 1:", t1, "result 1:", s.wait_results(1, 60))
time.sleep(1.0)
print("get_task_output (running) ->", (s.control("get_task_output", task_id=t1) or {}).get("response"))
print("background_tasks ->", (s.control("background_tasks") or {}).get("response"))
print("result 2 (self-started):", s.wait_results(2, 40))
time.sleep(1.0)
print("get_task_output (ended) ->", (s.control("get_task_output", task_id=t1) or {}).get("response"))
print("stop_task (ended) ->", (s.control("stop_task", task_id=t1) or {}).get("response"))

m = s.mark()
s.user("Run `sleep 127; echo late` with the Bash tool using run_in_background: true. Do not wait for it and do not check its output: right after starting it, reply with the single word: started")
t2 = task_id(m)
print("task 2:", t2, "result 3:", s.wait_results(3, 60))
time.sleep(2.0)
s.note(host_sees_sleep_127_before_stop=bool(alive("[s]leep 127")))
t = time.time()
print("stop_task (running) ->", (s.control("stop_task", task_id=t2) or {}).get("response"))
got = s.wait_results(4, 15)
print("result 4 after stop_task:", got, "%.1fs" % (time.time() - t))
s.note(host_sees_sleep_127_after_stop=bool(alive("[s]leep 127")), result_after_stop_task=got)
print("get_task_output (stopped) ->", (s.control("get_task_output", task_id=t2) or {}).get("response"))
print("stop_task (again) ->", (s.control("stop_task", task_id=t2) or {}).get("response"))
time.sleep(2)
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
print("sleep 127 left:", alive("[s]leep 127"))
