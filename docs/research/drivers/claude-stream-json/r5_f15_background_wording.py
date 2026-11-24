"""F15: prompt wording. A worker is told (in --append-system-prompt) that Codly stops background commands after 5
minutes and that it should stop them itself before its report. Does its turn end with the background command stopped?

The task needs a background command that never ends by itself (`sh tick_<run>.sh` writes one line per second to
ticks.log) and does not need it after the answer.

argv[1]: rule-1 | rule-2 | rule-3     with the sentence
         plain-1 | plain-2 | plain-3  control: the same task without the sentence
Weak evidence by design: three tiny runs per wording, one model, one task.
"""
import os, sys, time
from drv import Session, make_repo
from r5_lib import Procs, ask, open_tasks
variant = sys.argv[1] if len(sys.argv) > 1 else "rule-1"
RULE = ("Codly waits up to 5 minutes for what you left running in the background and then stops it: stop it yourself "
        "before your report unless the result is needed.")
name = "r5-background-wording-" + variant
cwd = make_repo(name)
script = "tick_%s.sh" % variant.replace("-", "_")
with open(os.path.join(cwd, script), "w") as f:
    f.write("while true; do date +%s >> ticks.log; sleep 1; done\n")
ticks = Procs("tick_%s.sh" % variant.replace("-", "_"))
system = "You are a worker in a team of coding agents. When your task is done, end your turn with a short report."
if variant.startswith("rule"):
    system += " " + RULE
s = Session(name, extra=["--tools", "Bash,Read,TaskStop", "--append-system-prompt", system], hard_timeout=200)
s.user("Start `sh %s` with the Bash tool using run_in_background: true (it writes one line per second to ticks.log and "
       "never ends by itself). When ticks.log has at least 3 lines, report its first line." % script)
print("result 1:", s.wait_results(1, 120))
with s.cv:
    tools = [(b.get("name"), str(b.get("input", {}).get("command") or b.get("input"))[:80]) for e in s.events if e.get("type") == "assistant"
             for b in e["message"]["content"] if b.get("type") == "tool_use"]
s.note(at_the_end_of_the_turn=dict(background_tasks_still_listed=open_tasks(s), tick_script_still_runs=bool(ticks.alive())),
       tool_calls=[t[0] for t in tools])
print("tools:", tools)
print("at the end of the turn: listed:", len(open_tasks(s)), "script alive:", bool(ticks.alive()))
time.sleep(8.0)
s.note(eight_seconds_later=dict(background_tasks_still_listed=open_tasks(s), tick_script_still_runs=bool(ticks.alive()),
                                results=s.count(lambda e: e.get("type") == "result")))
for t in open_tasks(s):  # clean up through the CLI, so that it can exit
    ask(s, "stop_task", task_id=t["task_id"])
print("cost:", s.cost(), "exit:", s.close())
print("killed leftovers:", ticks.kill_left())
