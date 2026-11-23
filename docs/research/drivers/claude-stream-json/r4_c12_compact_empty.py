"""C12: /compact as the very first message of a session, then after one tiny turn."""
import time
from drv import Session
s = Session("r4-compact-empty", extra=["--tools", "Bash"], hard_timeout=200)
n = 0
def turn(text, wait=60):
    global n
    m = s.mark()
    s.user(text)
    ok = s.wait_results(n + 1, wait)
    if ok:
        n += 1
    time.sleep(1.5)
    s.note(after=text, result_line=ok, lines=s.mark() - m)
    print("%-32s result=%s lines=%d" % (text, ok, s.mark() - m))
turn("/compact", 40)
turn("Reply with the single word: one")
turn("/compact", 60)
turn("Reply with the single word: two")
print("cost:", s.cost(), "exit:", s.close())
