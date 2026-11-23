"""A2: two user messages (with uuid) written while a turn runs.

argv[1]: tools (the turn has tool boundaries: three `sleep 4` calls) | notool (a text-only turn, no boundary)
"""
import sys, time, uuid
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "tools"
s = Session("r4-two-queued-" + variant, extra=["--tools", "Bash"], hard_timeout=150)
u = [str(uuid.uuid4()) for _ in range(3)]
if variant == "tools":
    s.user("Run the Bash command `sleep 4` three times, one tool call after the other (never in parallel, never combined in one command). After the third one finishes, reply with the single word: done", uuid=u[0])
    print("tool 1 started:", bool(s.wait_tool_use()))
    time.sleep(1.0)
else:
    s.user("Write the numbers from 1 to 150, one per line, and nothing else.", uuid=u[0])
    print("stream started:", bool(s.wait_for(lambda e: e.get("type") == "stream_event" and e["event"].get("type") == "content_block_delta", 60)))
s.user("also say the word banana at the end", uuid=u[1])
time.sleep(0.3)
s.user("and the word cherry after that", uuid=u[2])
print("result 1:", s.wait_results(1, 90))
print("result 2:", s.wait_results(2, 30))
print("result 3:", s.wait_results(3, 15))
time.sleep(3)
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
