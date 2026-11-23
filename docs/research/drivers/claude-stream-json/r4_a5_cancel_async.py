"""A5: cancel_async_message on a message that really waits in the queue, on one a turn already took, and twice."""
import time, uuid
from drv import Session
s = Session("r4-cancel-async-message", extra=["--tools", "Bash"], hard_timeout=150)
u = [str(uuid.uuid4()) for _ in range(3)]
s.user("Run the Bash command `sleep 6` two times, one tool call after the other (never in parallel, never combined in one command). After the second one finishes, reply with the single word: done", uuid=u[0])
print("tool 1 started:", bool(s.wait_tool_use()))
time.sleep(1.0)
s.user("also say the word banana at the end", uuid=u[1])
s.user("and the word cherry after that", uuid=u[2])
s.wait_for(lambda e: e.get("type") == "command_lifecycle" and e.get("command_uuid") == u[2] and e.get("state") == "queued", 10)
r = s.control("cancel_async_message", message_uuid=u[1])
print("cancel queued ->", r and r["response"])
r = s.control("cancel_async_message", message_uuid=u[1])
print("cancel again ->", r and r["response"])
# the other message is taken into the turn at the next tool boundary: cancel it after that
print("cherry started:", bool(s.wait_for(lambda e: e.get("type") == "command_lifecycle" and e.get("command_uuid") == u[2] and e.get("state") == "started", 30)))
r = s.control("cancel_async_message", message_uuid=u[2])
print("cancel taken ->", r and r["response"])
r = s.control("cancel_async_message", message_uuid=u[0])
print("cancel the turn's own message ->", r and r["response"])
print("result 1:", s.wait_results(1, 90))
print("result 2:", s.wait_results(2, 15))
time.sleep(2)
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
