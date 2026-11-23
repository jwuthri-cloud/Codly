"""A3: a helper that fails by itself, and a background command that is killed by a signal from outside.

argv[1]: helper-background  a custom agent (`--agents`) whose model name does not exist is started in the background:
                            its first model request fails. The task lines, the list line, the turn after it.
         helper-foreground  the same agent with run_in_background false: what the `Agent` call returns
         signal-term        turn 1 starts `echo begin; sleep 131; echo end` in the background. The driver finds the
                            shell that the CLI started for it (a child of the CLI whose command line has the command)
                            and sends it SIGTERM. The task lines, the list line, the turn after it.
         signal-kill        the same with SIGKILL
         signal-child-kill  SIGKILL to the `sleep 131` process alone (the child of that shell); the shell goes on
The helper runs cost nothing for the helper itself (its request is refused); the driver kills a `sleep 131` that the
signal left behind.
"""
import json, sys, time
from drv import Session
from r5_lib import Procs, open_tasks, results, task_started
from r6_lib import command_of, is_alive, kinds, parent_of, signal, stamps, system, wait_alive
variant = sys.argv[1] if len(sys.argv) > 1 else "helper-background"
name = "r6-fails-" + variant

if variant.startswith("helper"):
    agents = {"no-model": {"description": "A helper for tests.", "prompt": "You run the one command you are given and report its output.",
                           "model": "claude-no-such-model-0"}}
    s = Session(name, extra=["--tools", "Bash,Agent", "--agents", json.dumps(agents)], hard_timeout=150)
    if variant == "helper-background":
        s.user("Use the Agent tool with subagent_type `no-model` and run_in_background set to true to have that subagent run the Bash command "
               "`echo hi` and report its output. Do not wait for the subagent: right after starting it, reply with the single word: started")
    else:
        s.user("Use the Agent tool with subagent_type `no-model` and run_in_background set to false to have that subagent run the Bash command "
               "`echo hi` and report its output. Then tell me in one short sentence what happened. Do not try again.")
    t = task_started(s, task_type="local_agent", timeout=40)
    tid = t and t["task_id"]
    print("helper task:", tid, "is_backgrounded:", t and t.get("is_backgrounded"), "subagent_type:", t and t.get("subagent_type"))
    print("result 1:", s.wait_results(1, 60))
    end = s.wait_for(system("task_notification", task_id=tid), 40) if tid else None
    print("the helper's end:", end and end.get("status"))
    if variant == "helper-background":
        print("turn by itself:", s.wait_results(2, 40))
    time.sleep(1.0)
    with s.cv:
        helper_lines = [e for e in s.events if e.get("parent_tool_use_id") and e.get("type") in ("assistant", "user")]
    s.note(the_helper=dict(task_started=bool(t), is_backgrounded=t and t.get("is_backgrounded"), task_notification_status=end and end.get("status"),
                           lines=kinds(s), lines_of_the_helper=len(helper_lines), list=open_tasks(s), results=results(s)))
    print("lines:", kinds(s))
    print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
    sys.exit(0)

sig, target = {"signal-term": ("TERM", "shell"), "signal-kill": ("KILL", "shell"), "signal-child-kill": ("KILL", "child")}[variant]
sleeps = Procs("[s]leep 131")
s = Session(name, extra=["--tools", "Bash"], hard_timeout=150, own_group=True)
s.user("Run `echo begin; sleep 131; echo end` with the Bash tool using run_in_background: true. Do not wait for it and do not "
       "check its output: right after starting it, reply with the single word: started")
t = task_started(s, background=True)
tid = t and t["task_id"]
print("background task:", tid, "result 1:", s.wait_results(1, 60), "its sleep runs:", wait_alive(sleeps))
time.sleep(2.0)
# Two processes have the command in their command line: the shell that the CLI started for it (its parent is the CLI)
# and the `sleep` itself (its parent is that shell). Nothing else is ever signalled.
pids = sleeps.alive()
shells = [p for p in pids if parent_of(p) == str(s.p.pid) and "sleep 131" in command_of(p)]
children = [p for p in pids if parent_of(p) in shells]
s.note(before_the_signal=dict(list=open_tasks(s), processes_with_the_command=len(pids), shells_started_by_the_cli=len(shells), their_sleep_children=len(children)))
print("pids:", pids, "shells:", shells, "children:", children)
if len(shells) != 1 or len(children) != 1:
    print("killed leftovers:", sleeps.kill_left(), "exit:", s.close())
    raise SystemExit("the shell and its sleep were not found: the scenario did not reach its point")
victim = shells[0] if target == "shell" else children[0]
m, n = s.mark(), results(s)
t_signal = time.time() - s.t0
s.note(signal_sent=dict(signal="SIG" + sig, to="the shell that runs the background command" if target == "shell" else "the `sleep` process (a child of the command's shell)",
                        delivered=signal(victim, sig)))
end = s.wait_for(system("task_notification", task_id=tid), 30, m)
print("the command's end:", end and end.get("status"), end and end.get("summary"), "turn by itself:", s.wait_results(n + 1, 40))
time.sleep(1.0)
s.note(after_the_signal=dict(task_notification_status=end and end.get("status"), lines=kinds(s, m), list=open_tasks(s), shell_alive=is_alive(shells[0]),
                             sleep_alive=is_alive(children[0])),
       seconds_from_the_signal=stamps(s, t_signal, task_updated=system("task_updated"), task_notification=system("task_notification"),
                                      list_line=system("background_tasks_changed")))
print("lines after the signal:", kinds(s, m), "sleep still alive:", sleeps.alive())
print("killed leftovers:", sleeps.kill_left())
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
