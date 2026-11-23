"""Scenario 11: /compact, /context, /cost (and an unknown slash command) as user messages on a short session."""
import time
from drv import Session
s = Session("compaction-and-slash", extra=["--tools", "Bash"], hard_timeout=230)
n = 0
def turn(text, wait=90):
    global n
    n += 1
    m = s.mark()
    s.user(text)
    ok = s.wait_results(n, wait)
    time.sleep(1.0)
    print("%-28s result=%s lines=%d" % (text, ok, s.mark() - m))
turn("Run `echo one` with Bash, then reply with the single word: one")
turn("Reply with the single word: two")
turn("/context")
turn("/cost")
turn("/compact")
turn("Reply with the single word: three")
turn("/compact keep only the numbers")
turn("/this-command-does-not-exist")
turn("/clear")
turn("Reply with the single word: four")
print("exit:", s.close())
