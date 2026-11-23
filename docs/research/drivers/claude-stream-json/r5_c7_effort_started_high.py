"""C7: a process STARTED with `--effort high`; `apply_flag_settings` with another effort level, and with null.

get_settings is read after each step. No line of the stream names the effort of a model request, so each turn is a small
counting puzzle and the script notes the turn's thinking tokens (`result.usage.output_tokens_details`): two turns per
level. The CLI also writes a debug log (`--debug-file`, kept out of the repo); the script notes whether any of its
lines names an effort level.

argv[1]: high (default: started with --effort high) | none (control: started without --effort)
"""
import os, re, sys
from drv import Session, OUT
from r5_lib import ask, next_turn
variant = sys.argv[1] if len(sys.argv) > 1 else "high"
name = "r5-effort-started-" + variant
debug = os.path.join(OUT, "raw", name + ".debug.log")
if os.path.exists(debug):
    os.remove(debug)
extra = ["--tools", "Bash", "--debug-file", debug]
if variant == "high":
    extra += ["--effort", "high"]
s = Session(name, extra=extra, hard_timeout=280)
PUZZLES = iter([(300, 3, 7), (400, 3, 5), (500, 4, 7), (350, 5, 7), (450, 3, 11), (600, 7, 11), (250, 3, 13), (550, 5, 9)])

def applied():
    return ask(s, "get_settings").get("response", {}).get("applied", {}).get("effort")

def turns(level):
    """Two puzzle turns; the note has what get_settings says and what each turn's result says about thinking."""
    for _ in range(2):
        n, a, b = next(PUZZLES)
        before = applied()
        ok = next_turn(s, "How many integers from 1 to %d are divisible by %d or by %d, but not by both? Reply with the number only." % (n, a, b), 90)
        with s.cv:
            r = [e for e in s.events if e.get("type") == "result"][-1]
        u = r.get("usage", {})
        s.note(level=level, applied_effort_before_the_turn=before, applied_effort_after_the_turn=applied(), turn_ok=ok,
               thinking_tokens=u.get("output_tokens_details", {}).get("thinking_tokens"), output_tokens=u.get("output_tokens"))
        print("%-22s applied=%s thinking_tokens=%s output_tokens=%s answer=%r" % (level, before, u.get("output_tokens_details", {}).get("thinking_tokens"),
                                                                             u.get("output_tokens"), str(r.get("result"))[:40]))

s.control("initialize", hooks=None)
turns("as started")
ask(s, "apply_flag_settings", settings={"effortLevel": "low"})
turns("effortLevel low")
ask(s, "apply_flag_settings", settings={"effortLevel": None})
turns("effortLevel null")
ask(s, "apply_flag_settings", settings={"effortLevel": "high"})
turns("effortLevel high")
log = open(debug, errors="replace").read() if os.path.exists(debug) else ""
named = [l for l in log.splitlines() if re.search(r"effort\W{1,4}(low|medium|high|xhigh|max)\b", l)]
s.note(debug_log_lines=log.count("\n"), debug_log_lines_that_name_an_effort_level=len(named))
print("debug log: %d lines, %d name an effort level" % (log.count("\n"), len(named)))
print("cost:", s.cost(), "exit:", s.close())
