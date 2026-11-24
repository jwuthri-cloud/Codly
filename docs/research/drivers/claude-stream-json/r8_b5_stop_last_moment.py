"""B5: `stop_task` in the last moment of a turn, and `stop_task` on a background COMMAND while a turn runs. Part 5
(section 11) stopped a helper while a tool call of the running turn ran: one model request of the turn followed, it
took the notification, and no turn started by itself.

argv[1]: helper-last-request   turn 1 starts a helper in the background (its Bash call: `echo begin; sleep 211; echo
                               end`) and ends. Turn 2 asks for a poem with no tool: ONE model request, the turn's last.
                               At its first streamed text delta the host sends `stop_task` with the helper's id.
         command-mid-turn      turn 1 starts a background command (`echo bg; sleep 223`, run_in_background) and ends.
                               Turn 2 runs `echo go; sleep 22.5` in the foreground; 2 s into it (counted from the moment
                               the driver sees its `sleep`) the host sends `stop_task` with the command's id.
         command-last-request  turn 1 as in command-mid-turn (`sleep 227`); turn 2 asks for the poem; `stop_task` with
                               the command's id at its first streamed text delta.
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.

The host reads until turn 2's result and then 25 s more, and notes every line since the request (the note under the
driver's lock, in the same step as the list; process checks before it). After the exit a note counts, from the
session's own transcript, the task notifications that waited in the CLI's queue (r7_lib.queued_notifications).
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, open_tasks, results, task_started
from r6_lib import answer_to, is_init, is_result, kinds, sent_at, stamps, system, tool_call, wait_alive
from r7_lib import queued_notifications
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "helper-last-request"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
BG = {"helper-last-request": "sleep 211", "command-mid-turn": "sleep 223", "command-last-request": "sleep 227"}[VARIANT]
bg, fg = Procs(BG), Procs("sleep 22.5")
s = Session("r8-stop-%s-%s" % (VARIANT, LABEL), extra=["--tools", "Bash,Agent"], hard_timeout=200)
if VARIANT.startswith("helper"):
    s.user("Use the Agent tool with run_in_background set to true to have a subagent run the Bash command "
           "`echo begin; %s; echo end` and report its output. Do not wait for the subagent: right after starting it, "
           "reply with the single word: started" % BG)
    t = task_started(s, task_type="local_agent")
else:
    s.user("Run the Bash command `echo bg; %s` with run_in_background set to true. Do not wait for it: right after starting it, "
           "reply with the single word: started" % BG)
    t = task_started(s, task_type="local_bash", background=True)
tid = t and t["task_id"]
print("task:", tid, "result 1:", s.wait_results(1, 60), "its sleep runs:", wait_alive(bg, 30))
m = s.mark()


def main_text_delta(e):
    ev = e.get("event") or {}
    return (e.get("type") == "stream_event" and not e.get("parent_tool_use_id") and ev.get("type") == "content_block_delta"
            and (ev.get("delta") or {}).get("type") == "text_delta")


if VARIANT.endswith("last-request"):
    s.user("Write a poem of about 250 words about the sea. Use no tools.")
    when = "at the first streamed text delta of turn 2"
    print("text streams:", bool(s.wait_for(main_text_delta, 60, m)))
else:
    s.user("Run the Bash command `echo go; sleep 22.5` in the foreground and then reply with the single word: done")
    when = "2 s after the sleep of turn 2's Bash call was seen"
    print("the turn's Bash call began:", bool(tool_call(s, "Bash", helper=False, start=m)), "its sleep runs:", wait_alive(fg))
    time.sleep(2.0)
alive = dict(bg_sleep_alive=bool(bg.alive()), fg_sleep_alive=bool(fg.alive()))
with s.cv:
    s.note(before_stop_task=dict(when=when, list=open_tasks(s), a_turn_runs=results(s) == 1, **alive))
m, n = s.mark(), results(s)
ask(s, "stop_task", task_id=tid)
stop_answer = answer_to(s)
print("turn 2's result:", s.wait_results(n + 1, 90))
time.sleep(25.0)
alive = dict(bg_sleep_alive=bool(bg.alive()), fg_sleep_alive=bool(fg.alive()))
with s.cv:
    r = [e for e in s.events[m:] if e.get("type") == "result"]
    s.note(after_stop_task=dict(lines=kinds(s, m), results_since_the_request=len(r), result_origins=[e.get("origin") for e in r], list=open_tasks(s), **alive),
           seconds_from_stop_task=stamps(s, sent_at(s, "stop_task"), answer=stop_answer, task_notification=system("task_notification", task_id=tid),
                                         result=is_result, init_after_it=is_init))
print("lines since stop_task:", kinds(s, m))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close(after_exit=lambda: dict(transcript=queued_notifications(s.sid))))
print("killed leftovers:", bg.kill_left(), fg.kill_left())
