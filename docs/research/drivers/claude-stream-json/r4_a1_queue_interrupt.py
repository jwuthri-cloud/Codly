"""A1: a user message (with uuid) waits in the CLI's queue; then the host sends `interrupt`.

argv[1]: plain (interrupt with no fields) | cancel-queued (interrupt with cancel_queued:true)
"""
import sys, time, uuid
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "plain"
s = Session("r4-queue-interrupt-" + variant, extra=["--tools", "Bash"], hard_timeout=120)
u1, u2 = str(uuid.uuid4()), str(uuid.uuid4())
s.user("Run the Bash command `sleep 15` and then reply with the single word: done", uuid=u1)
print("tool started:", bool(s.wait_tool_use()))
time.sleep(1.0)
s.user("Reply with the single word: banana", uuid=u2)
print("queued:", bool(s.wait_for(lambda e: e.get("type") == "command_lifecycle" and e.get("command_uuid") == u2 and e.get("state") == "queued", 10)))
time.sleep(1.0)
t = time.time()
r = s.control("interrupt", **({"cancel_queued": True} if variant == "cancel-queued" else {}))
print("interrupt ->", r and r["response"])
print("result 1:", s.wait_results(1, 30), "%.2fs after the interrupt" % (time.time() - t))
print("result 2:", s.wait_results(2, 30), "%.2fs after the interrupt" % (time.time() - t))
time.sleep(4)
s.note(results_seen=s.count(lambda e: e.get("type") == "result"))
# is the process still usable?
s.user("Reply with the single word: alive", uuid=str(uuid.uuid4()))
n = s.count(lambda e: e.get("type") == "result")
print("next turn:", s.wait_results(n + 1, 40))
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
