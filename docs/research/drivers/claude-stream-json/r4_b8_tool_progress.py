"""B8: which lines arrive while a long tool call (`sleep 20`) runs, and how fast an interrupt mid-tool ends the turn."""
import subprocess, time
from drv import Session
s = Session("r4-tool-progress", extra=["--tools", "Bash"], hard_timeout=170)

def sleeping():
    return [p for p in subprocess.run(["pgrep", "-f", "[s]leep 2[01]$"], capture_output=True, text=True).stdout.split() if p not in BEFORE]

BEFORE = ()
BEFORE = sleeping()  # processes of other programs that happen to match: never counted

s.user("Run the Bash command `sleep 20` and then reply with the single word: one")
print("result 1:", s.wait_results(1, 60))
m = s.mark()
s.user("Run the Bash command `sleep 21` and then reply with the single word: two")
print("tool started:", bool(s.wait_tool_use(m)))
time.sleep(8.0)
s.note(sleep_21_alive_before_interrupt=bool(sleeping()))
t = time.time()
r = s.control("interrupt")
t_resp = time.time() - t
ok = s.wait_results(2, 30)
t_res = time.time() - t
print("interrupt response after %.3fs, result after %.3fs" % (t_resp, t_res), ok)
time.sleep(1.0)
s.note(interrupt_response_after_s=round(t_resp, 3), result_after_s=round(t_res, 3), sleep_21_alive_1s_after_interrupt=bool(sleeping()))
print("results:", s.count(lambda e: e.get("type") == "result"), "cost:", s.cost())
print("exit:", s.close())
print("left:", sleeping())
