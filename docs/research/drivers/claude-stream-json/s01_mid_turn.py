"""Scenario 1: a second user message written while tool 1 of a 3-tool turn runs.

Variants (argv[1]): plain (no uuid, as the adapter sends today), uuid (both messages carry a client uuid),
uuid-now / uuid-next / uuid-later (second message also carries `priority`).
"""
import sys, time, uuid
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "plain"
s = Session("mid-turn-user-message" + ("" if variant == "plain" else "-" + variant), extra=["--tools", "Bash"], hard_timeout=150, repo="mid-turn-" + variant)
e1, e2 = {}, {}
if variant.startswith("uuid"):
    e1["uuid"] = str(uuid.uuid4())
    e2["uuid"] = str(uuid.uuid4())
if "-" in variant:
    e2["priority"] = variant.split("-")[1]
s.user("Run the Bash command `sleep 4` three times, one tool call after the other (never in parallel, never combined in one command). After the third one finishes, reply with the single word: done", **e1)
# wait until the first Bash tool_use is complete (assistant message with tool_use), i.e. tool 1 is running
ev = s.wait_for(lambda e: e.get("type") == "assistant" and any(b.get("type") == "tool_use" for b in e["message"]["content"]), 60)
print("tool 1 started:", bool(ev))
time.sleep(1.0)
s.user("also say the word banana at the end", **e2)
s.wait_results(1, 90)
got2 = s.wait_results(2, 30)  # keep reading: does a second result come?
print("second result:", got2)
time.sleep(3)
print("results:", s.count(lambda e: e.get("type") == "result"))
print("exit:", s.close())
