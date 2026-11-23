"""C4: `apply_flag_settings {"effortLevel": null}` as the FIRST such request on a process started with `--effort high`.

Part 3 (section 7) sent `null` after an `effortLevel:"low"`; here nothing was applied before it.
get_settings is read after each step; the note of each step has `applied.effort` and what `effective` and `sources`
say about `effortLevel`.

argv[1]: fresh       initialize, get_settings, null, get_settings, one turn, get_settings, "high", get_settings
         after-turn  the same with one turn before the null
"""
import sys
from drv import Session
from r5_lib import ask, next_turn
variant = sys.argv[1] if len(sys.argv) > 1 else "fresh"
s = Session("r6-effort-null-first-" + variant, extra=["--tools", "Bash", "--effort", "high"], hard_timeout=120)


def read(step):
    r = ask(s, "get_settings").get("response", {})
    named = [x.get("source") for x in r.get("sources", []) if "effortLevel" in (x.get("settings") or {})]
    s.note(step=step, applied_effort=r.get("applied", {}).get("effort"), effective_effortLevel=r.get("effective", {}).get("effortLevel", "(no such key)"),
           sources_that_name_effortLevel=named)
    print("%-34s applied.effort=%s" % (step, r.get("applied", {}).get("effort")))


s.control("initialize", hooks=None)
read("as started")
if variant == "after-turn":
    print("turn:", next_turn(s, "Reply with the single word: one"))
    read("after a turn")
ask(s, "apply_flag_settings", settings={"effortLevel": None})
read("after effortLevel null")
print("turn:", next_turn(s, "Reply with the single word: two"))
read("after the next turn")
ask(s, "apply_flag_settings", settings={"effortLevel": "high"})
read("after effortLevel high")
print("cost:", s.cost(), "exit:", s.close())
