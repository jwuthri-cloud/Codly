"""A4: the stop sequence as one run, an interrupt while THREE messages wait, and an interrupt after a message was
folded into the turn.

argv[1]: sequence       a background command runs; a turn runs a tool call; a message waits; then
                        interrupt{cancel_queued:true} -> (the turn's result) -> stop_task -> stdin closed
         three-plain    a turn runs `sleep 15`; three messages wait; plain interrupt
         three-cancel   the same with cancel_queued:true
         folded-plain   a message is folded into the turn at a tool boundary; then a plain interrupt mid-tool
         folded-cancel  the same with cancel_queued:true
The CLI is started as the leader of its own process group, as the app starts it.
"""
import sys, time, uuid
from drv import Session
from r5_lib import Procs, ask, next_turn, open_tasks, results, task_started
variant = sys.argv[1] if len(sys.argv) > 1 else "sequence"
s = Session("r5-stop-" + variant, extra=["--tools", "Bash"], hard_timeout=150, own_group=True)

def lifecycle(u, state, timeout=15):
    return bool(s.wait_for(lambda e: e.get("type") == "command_lifecycle" and e.get("command_uuid") == u and e.get("state") == state, timeout))

def states():
    with s.cv:
        return [[names.get(e["command_uuid"], "?"), e["state"]] for e in s.events if e.get("type") == "command_lifecycle"]

names = {}
def new(label):
    u = str(uuid.uuid4())
    names[u] = label
    return u

cancel = {"cancel_queued": True} if variant.endswith("cancel") or variant == "sequence" else {}

if variant == "sequence":
    late, fg = Procs("sleep 307"), Procs("sleep 61")
    s.user("Run `sleep 307; echo late` with the Bash tool using run_in_background: true. Do not wait for it and do not "
           "check its output: right after starting it, reply with the single word: started", uuid=new("m0"))
    t = task_started(s, background=True)
    tid = t and t["task_id"]
    print("background task:", tid, "result 1:", s.wait_results(1, 60))
    m = s.mark()
    s.user("Run the Bash command `echo go; sleep 61` and then reply with the single word: done", uuid=new("m1"))
    print("tool started:", bool(s.wait_tool_use(m, name="Bash")))
    time.sleep(1.0)
    u2 = new("m2")
    s.user("Reply with the single word: banana", uuid=u2)
    print("m2 queued:", lifecycle(u2, "queued"))
    time.sleep(1.0)
    s.note(before_the_stop=dict(sleep_307_alive=bool(late.alive()), sleep_61_alive=bool(fg.alive()), list=open_tasks(s)),
           lifecycle_before_interrupt=states(), uuids={v: k for k, v in names.items()})
    t0 = time.time()
    ask(s, "interrupt", **cancel)
    got = s.wait_results(2, 30)
    t_result = time.time() - t0
    ask(s, "stop_task", task_id=tid)
    t_stopped = time.time() - t0
    s.note(result_after_interrupt_s=round(t_result, 3), stop_task_answered_after_s=round(t_stopped, 3), interrupted_turn_result=got,
           list_after_stop_task=open_tasks(s))
    def after_exit():
        time.sleep(1.0)
        return dict(after_the_exit=dict(sleep_307_alive=bool(late.alive()), sleep_61_alive=bool(fg.alive())))
    code = s.close(wait=30, after_exit=after_exit)
    print("result %.3fs, stop_task answered %.3fs after the interrupt; exit code %s" % (t_result, t_stopped, code))
    print("results:", results(s), "lifecycle:", states())
    print("left: sleep 307", late.alive(), "sleep 61", fg.alive())
    print("killed leftovers:", late.kill_left(), fg.kill_left())
    sys.exit(0)

if variant.startswith("three"):
    s.user("Run the Bash command `sleep 15` and then reply with the single word: done", uuid=new("m1"))
    print("tool started:", bool(s.wait_tool_use(name="Bash")))
    time.sleep(1.0)
    for label, word in (("m2", "banana"), ("m3", "cherry"), ("m4", "damson")):
        u = new(label)
        s.user("Reply with the single word: " + word, uuid=u)
        print(label, "queued:", lifecycle(u, "queued"))
        time.sleep(0.3)
else:
    s.user("Run the Bash command `sleep 4`. After it finished, run the Bash command `echo go; sleep 21` in a second, "
           "separate tool call. Then reply with the single word: done", uuid=new("m1"))
    print("first tool started:", bool(s.wait_tool_use(name="Bash")))
    time.sleep(1.0)
    u2 = new("m2")
    s.user("When you are done, also say the word banana.", uuid=u2)
    print("m2 queued:", lifecycle(u2, "queued"), "m2 started (folded):", lifecycle(u2, "started", 30))
    second = s.wait_for(lambda e: e.get("type") == "assistant" and any(
        b.get("type") == "tool_use" and "sleep 21" in str(b.get("input", {}).get("command")) for b in e["message"]["content"]), 40)
    print("second tool started:", bool(second))
    time.sleep(2.0)
s.note(lifecycle_before_interrupt=states(), uuids={v: k for k, v in names.items()})
n = results(s)
r = ask(s, "interrupt", **cancel)
print("result of the interrupted turn:", s.wait_results(n + 1, 30))
time.sleep(12.0)
s.note(results_in_the_12s_after_the_interrupt=results(s) - n, lifecycle=states())
print("results after the interrupt:", results(s) - n, "lifecycle:", states())
print("next turn:", next_turn(s, "Reply with the single word: alive", 40, uuid=new("m9")))
time.sleep(1.0)
s.note(lifecycle_at_the_end=states())
print("results:", results(s), "cost:", s.cost())
print("exit:", s.close())
