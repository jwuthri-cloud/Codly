"""Scenario 5: a background Bash command that ends after the turn. Does the CLI start a turn by itself?"""
import time
from drv import Session
s = Session("background-wakeup", extra=["--tools", "Bash"], hard_timeout=150)
s.user("Run `sleep 6; echo woke` with the Bash tool using run_in_background: true. Do not wait for it and do not check its output: right after starting it, reply with the single word: started")
print("result 1:", s.wait_results(1, 60))
t = time.time()
got2 = s.wait_results(2, 40)
print("second result:", got2, "after %.1fs" % (time.time() - t))
time.sleep(max(0, 30 - (time.time() - t)))
print("results:", s.count(lambda e: e.get("type") == "result"))
print("exit:", s.close())
