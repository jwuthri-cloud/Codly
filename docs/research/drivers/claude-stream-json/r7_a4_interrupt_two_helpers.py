"""A4: an `interrupt` with TWO background helpers at work, and with a helper that waits for a model answer (not in a
Bash call). No turn of the main agent runs when the interrupt comes (part 4, section 1, recorded one helper, always
inside a Bash call).

argv[1]: two    turn 1 starts two helpers in the background (their Bash calls: `echo a; sleep 181; echo end` and
                `echo b; sleep 191; echo end`) and ends. 3 s after the driver sees both `sleep` processes the host
                sends `interrupt` and reads for 10 s. Then one more turn.
         model  turn 1 starts one helper in the background that writes a long story with no tool, and ends. The
                CLI streams no partial lines of a helper, so the host sends `interrupt` 1.5 s after the result of
                turn 1 (writing about 800 words takes the model longer) and reads for 10 s. Then one more turn. The
                note before the interrupt lists the block types of the helper's assistant lines so far: no text
                block and the helper still listed = it was still waiting for its answer (else record again).
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, next_turn, open_tasks, results, task_started
from r6_lib import answer_to, is_init, is_result, kinds, sent_at, stamps, system, wait_alive
variant = sys.argv[1] if len(sys.argv) > 1 else "two"
run = sys.argv[2] if len(sys.argv) > 2 else "1"
s = Session("r7-interrupt-%s-%s" % ("two-helpers" if variant == "two" else "helper-model", run), extra=["--tools", "Bash,Agent"], hard_timeout=200)
a, b = Procs("[s]leep 181"), Procs("[s]leep 191")


def helper_ids():
    with s.cv:
        return [e["task_id"] for e in s.events if e.get("type") == "system" and e.get("subtype") == "task_started" and e.get("task_type") == "local_agent"]


if variant == "two":
    s.user("Use the Agent tool twice, both times with run_in_background set to true, in one message: the first subagent runs the "
           "Bash command `echo a; sleep 181; echo end` and reports its output, the second runs `echo b; sleep 191; echo end` and "
           "reports its output. Do not wait for them: right after starting both, reply with the single word: started")
    print("result 1:", s.wait_results(1, 60))
    print("both sleeps run:", wait_alive(a, 30), wait_alive(b, 30))
    time.sleep(3.0)
    how = "3 s after both sleep processes were seen"
else:
    s.user("Use the Agent tool with run_in_background set to true to have a subagent write a story of about 800 words about a "
           "lighthouse keeper, using no tools at all, and report the story. Do not wait for the subagent: right after starting it, "
           "reply with the single word: started")
    t = task_started(s, task_type="local_agent")
    tool_use_id = t and t.get("tool_use_id")
    print("result 1:", s.wait_results(1, 60))
    time.sleep(1.5)
    how = "1.5 s after the result of turn 1"
    print("helper:", t and t["task_id"], how)
with s.cv:
    helper_lines = [e for e in s.events if e.get("type") == "assistant" and e.get("parent_tool_use_id")]
s.note(before_the_interrupt=dict(when=how, helpers=len(helper_ids()), list=open_tasks(s), a_turn_runs=results(s) == 0,
                                 assistant_lines_of_helpers=[[b["type"] for b in e["message"]["content"]] for e in helper_lines],
                                 sleep_181_alive=bool(a.alive()), sleep_191_alive=bool(b.alive())))
m, n = s.mark(), results(s)
ask(s, "interrupt")
time.sleep(10.0)
alive = dict(sleep_181_alive=bool(a.alive()), sleep_191_alive=bool(b.alive()))
ends = {}
with s.cv:
    for e in s.events[m:]:
        if e.get("type") == "system" and e.get("subtype") in ("task_updated", "task_notification") and e.get("task_id") in helper_ids():
            ends.setdefault(e["task_id"], []).append(e.get("status") or e.get("patch", {}).get("status"))
    s.note(after_the_interrupt=dict(lines=kinds(s, m), end_words_per_helper=list(ends.values()), helpers_ended=len(ends), results_in_the_10s_after=results(s) - n,
                                    list=open_tasks(s), **alive),
           seconds_from_the_interrupt_request=stamps(s, sent_at(s, "interrupt"), answer=answer_to(s), first_task_notification=system("task_notification"),
                                                     result=is_result, next_init=is_init))
print("lines after the interrupt:", kinds(s, m))
print("next turn:", next_turn(s, "Reply with the single word: alive", 40))
time.sleep(1.0)
s.note(at_the_end=dict(list=open_tasks(s), results=results(s), sleep_181_alive=bool(a.alive()), sleep_191_alive=bool(b.alive())))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
print("killed leftovers:", a.kill_left(), b.kill_left())
