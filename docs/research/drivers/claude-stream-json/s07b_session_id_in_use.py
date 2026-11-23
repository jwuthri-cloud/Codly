"""Scenario 7 (bonus): --session-id with an id that already has a transcript (the smoke session)."""
import json, os, time
from drv import Session, OUT
first = json.loads(open(os.path.join(OUT, "raw", "smoke.ndjson")).readline())
sid = first["msg"]["argv"][first["msg"]["argv"].index("--session-id") + 1]
s = Session("session-id-in-use", extra=["--session-id", sid, "--tools", "Bash"], session_id=None, hard_timeout=60, repo="smoke")
time.sleep(4)
print("alive before any input:", s.alive())
if s.alive():
    s.user("Reply with the single word: pong")
    print("result:", s.wait_results(1, 30))
print("exit:", s.close())
