"""A3: a slash command written (with uuid) while a turn runs.  argv[1]: context | compact"""
import sys, time, uuid
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "context"
s = Session("r4-slash-mid-turn-" + variant, extra=["--tools", "Bash"], hard_timeout=150)
u1, u2 = str(uuid.uuid4()), str(uuid.uuid4())
s.user("Run the Bash command `sleep 4` two times, one tool call after the other (never in parallel, never combined in one command). After the second one finishes, reply with the single word: done", uuid=u1)
print("tool 1 started:", bool(s.wait_tool_use()))
time.sleep(1.0)
s.user("/" + variant, uuid=u2)
print("result 1:", s.wait_results(1, 90))
print("result 2:", s.wait_results(2, 60))
print("result 3:", s.wait_results(3, 10))
time.sleep(2)
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
