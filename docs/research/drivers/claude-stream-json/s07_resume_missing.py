"""Scenario 7: --resume <random uuid> for a session that does not exist."""
import time, uuid
from drv import Session
rid = str(uuid.uuid4())
s = Session("resume-missing", extra=["--resume", rid, "--tools", "Bash"], session_id=None, hard_timeout=60)
time.sleep(4)
print("alive before any input:", s.alive())
if s.alive():
    s.user("Reply with the single word: pong")
    print("result:", s.wait_results(1, 30))
    time.sleep(2)
print("alive:", s.alive())
print("exit:", s.close())
