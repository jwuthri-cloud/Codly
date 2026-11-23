"""C8: `apply_flag_settings {"effortLevel": null}` on a process started with `--effort low` or `--effort max`.

Part 4 (section 4) sent the same request to a `--effort high` process: `applied.effort` became `medium`.
Order: initialize, get_settings, null, get_settings, one turn, get_settings. The note of each step has
`applied.effort` and what `effective` and `sources` say about `effortLevel`.

argv[1]: low | max   the level the process is started with
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys
from drv import Session
from r5_lib import ask, next_turn
level = sys.argv[1] if len(sys.argv) > 1 else "low"
run = sys.argv[2] if len(sys.argv) > 2 else "1"
s = Session("r7-effort-null-%s-%s" % (level, run), extra=["--tools", "Bash", "--effort", level], hard_timeout=120)


def read(step):
    r = ask(s, "get_settings").get("response", {})
    named = [x.get("source") for x in r.get("sources", []) if "effortLevel" in (x.get("settings") or {})]
    s.note(step=step, applied_effort=r.get("applied", {}).get("effort"), effective_effortLevel=r.get("effective", {}).get("effortLevel", "(no such key)"),
           sources_that_name_effortLevel=named)
    print("%-34s applied.effort=%s" % (step, r.get("applied", {}).get("effort")))


s.control("initialize", hooks=None)
read("as started")
ask(s, "apply_flag_settings", settings={"effortLevel": None})
read("after effortLevel null")
print("turn:", next_turn(s, "Reply with the single word: two"))
read("after the next turn")
print("cost:", s.cost(), "exit:", s.close())
