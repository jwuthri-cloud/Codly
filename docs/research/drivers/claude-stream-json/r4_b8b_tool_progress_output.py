"""B8b: a 96 s foreground Bash command that prints a line every 4 s. Which lines arrive while it runs?
Mid-run the host reads it with get_task_output (the task id comes from task_started)."""
import time
from drv import Session
s = Session("r4-tool-progress-output", extra=["--tools", "Bash"], hard_timeout=200)
s.user("Run exactly this Bash command in the foreground (one call, as written) and then reply with the single word: done\n\nfor i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24; do echo tick $i; sleep 4; done")
print("tool started:", bool(s.wait_tool_use()))
ev = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "task_started", 30)
print("task_started:", bool(ev))
if ev:
    time.sleep(6)
    r = s.control("get_task_output", task_id=ev["task_id"])
    print("get_task_output ->", r and r["response"])
print("result:", s.wait_results(1, 150))
print("tool_progress lines:", s.count(lambda e: e.get("type") == "tool_progress"))
print("cost:", s.cost(), "exit:", s.close())
