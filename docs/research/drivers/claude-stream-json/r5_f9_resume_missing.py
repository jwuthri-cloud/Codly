"""F9: `--resume <unknown id>` with an `initialize` request written at once, as a host does. Five runs
(r5-resume-missing-1 … -5): is the `initialize` answer written before the process gives up, and what are the last lines?
No message is sent; the runs cost nothing.
"""
import time, uuid
from drv import Session
for i in range(1, 6):
    s = Session("r5-resume-missing-%d" % i, extra=["--resume", str(uuid.uuid4()), "--tools", "Bash"], session_id=None, hard_timeout=60,
                repo="r5-resume-missing")
    s.control("initialize", wait=0, hooks=None)
    end = time.time() + 20
    while s.alive() and time.time() < end:
        time.sleep(0.05)
    time.sleep(0.5)
    with s.cv:
        kinds = ["initialize answer (%s)" % e["response"].get("subtype") if e.get("type") == "control_response"
                 else "%s/%s" % (e.get("type"), e.get("subtype")) for e in s.events]
    s.note(stdout_lines_in_order=kinds, exited_by_itself=not s.alive())
    print("run %d: stdout lines in order: %s; exited by itself: %s; exit code: %s" % (i, kinds, not s.alive(), s.close()))
