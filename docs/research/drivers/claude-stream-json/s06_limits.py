"""Scenario 6: --max-turns 1 and --max-budget-usd 0.0001.

argv[1]: max-turns | max-turns-then-close (no second message, exit code after the error result) | max-budget
"""
import sys, time
from drv import Session
which = sys.argv[1]
if which.startswith("max-turns"):
    s = Session("limits-" + which, extra=["--tools", "Bash", "--max-turns", "1"], hard_timeout=120)
    s.user("Run `echo one` with Bash. After you see its output, run `echo two` with Bash in a second, separate tool call. Then reply: done")
else:
    s = Session("limits-max-budget", extra=["--tools", "Bash", "--max-budget-usd", "0.0001"], hard_timeout=120)
    s.user("Run `echo one` with Bash. After you see its output, run `echo two` with Bash in a second, separate tool call. Then reply: done")
print("result 1:", s.wait_results(1, 60))
time.sleep(3)
print("alive 3s after result:", s.alive())
if s.alive() and which != "max-turns-then-close":
    s.user("Reply with the single word: alive")
    print("result 2:", s.wait_results(2, 40))
    time.sleep(2)
    print("alive after 2nd message:", s.alive())
print("exit:", s.close())
