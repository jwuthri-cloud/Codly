"""C12: `apply_flag_settings` with an effort level WHILE a turn runs. Parts 4 and 5 sent it with no turn running.

The process starts with `--effort low`. Turn 1 runs `echo go; sleep 13.5` in the foreground and then answers; 1 s after
the driver sees the `sleep`, the host sends get_settings, then the variant's request, then get_settings again (all
while the call runs), and get_settings after the turn's result and after one more turn. Each step's note has
`applied.effort`, whether the turn still ran, and the answer's subtype.

argv[1]: null    `apply_flag_settings {"effortLevel": null}`
         high    `apply_flag_settings {"effortLevel": "high"}`
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, next_turn, results
from r6_lib import raw, tool_call, wait_alive
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "null"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
LEVEL = {"null": None, "high": "high"}[VARIANT]
call = Procs("sleep 13.5")
s = Session("r8-effort-mid-turn-%s-%s" % (VARIANT, LABEL), extra=["--tools", "Bash", "--effort", "low"], hard_timeout=120)


def turn_runs():
    """A message of the host has no result yet."""
    return sum(1 for r in raw(s) if r["dir"] == "send" and r["msg"].get("type") == "user") > results(s)


def read(step):
    r = ask(s, "get_settings").get("response", {})
    s.note(step=step, applied_effort=r.get("applied", {}).get("effort"), a_turn_runs=turn_runs(), sleep_13_5_alive=bool(call.alive()))


s.control("initialize", hooks=None)
read("as started")
m = s.mark()
s.user("Run the Bash command `echo go; sleep 13.5` in the foreground and then reply with the single word: done")
print("the call began:", bool(tool_call(s, "Bash", helper=False, start=m)), "its sleep runs:", wait_alive(call, 30))
time.sleep(1.0)
read("during the call, before the request")
r = ask(s, "apply_flag_settings", settings={"effortLevel": LEVEL})
s.note(step="apply_flag_settings", answer_subtype=r.get("subtype"), a_turn_runs=turn_runs(), sleep_13_5_alive=bool(call.alive()))
read("during the call, after the request")
print("turn 1:", s.wait_results(1, 60))
read("after the turn's result")
print("turn 2:", next_turn(s, "Reply with the single word: two"))
read("after the next turn")
print("cost:", s.cost(), "exit:", s.close())
print("killed leftovers:", call.kill_left())
