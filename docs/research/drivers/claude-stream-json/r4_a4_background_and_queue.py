"""A4: a background Bash task ends while a user message waits in the queue.

Turn 1 starts `sleep 12; echo woke` in the background. Turn 2 runs a foreground `sleep 14`; one second into it a user
message is written; the background task ends while that message waits.
argv[1]: fold (no priority: the message goes into the running turn) | later (priority "later": it waits for the turn's end)
"""
import sys, time, uuid
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "fold"
s = Session("r4-background-and-queue-" + variant, extra=["--tools", "Bash"], hard_timeout=170)
u = [str(uuid.uuid4()) for _ in range(3)]
s.user("Run `sleep 12; echo woke` with the Bash tool using run_in_background: true. Do not wait for it and do not check its output: right after starting it, reply with the single word: started", uuid=u[0])
print("result 1:", s.wait_results(1, 60))
m = s.mark()
s.user("Run the Bash command `sleep 14` (in the foreground) and then reply with the single word: two", uuid=u[1])
print("foreground tool started:", bool(s.wait_tool_use(m)))
time.sleep(1.0)
s.user("Reply with the single word: three", uuid=u[2], **({"priority": "later"} if variant == "later" else {}))
print("result 2:", s.wait_results(2, 60))
print("result 3:", s.wait_results(3, 40))
print("result 4:", s.wait_results(4, 25))
time.sleep(3)
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
