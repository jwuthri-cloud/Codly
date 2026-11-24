"""F6: after a budget stop (`error_max_budget_usd`). Part 5 (section 10): the limit passed by a helper's request while
the turn's foreground Bash call ran ends the TURN, not the call; the call runs on, unlisted, until stdin is closed.

argv[1]: interrupt     as part 5: a helper writes a story of about 600 words with no tool (one long request) while the
                       main thread runs `echo go; sleep 53.5`. After the stop (the call still running) the host waits
                       2 s and sends `interrupt`, stdin still open, and reads 5 s.
         stop-task     the same (`sleep 57.5`); the host sends `stop_task` with the task id of the call (its
                       `task_started` line: a foreground call becomes a task after about 3 s, part 5 section 12).
         helper-goes-on  the helper runs four Bash calls one after the other (`echo h1` … `echo h4`), so it still has
                       model requests to make when the limit passes; the main thread runs `echo go; sleep 47.5`. After
                       the stop the host reads 40 s: the helper's tool calls after the stop, its end, every `result`
                       (subtype, origin, total_cost_usd).
argv[2]: a label for the recording (1, 2): the whole scenario, recorded more than once.

Run calib (r8-after-budget-<variant>-<label>-calib): no budget. Tries t1, t2, … : `--max-budget-usd` = the cost when the
Bash call began + a share of the helper's cost (half for the story; a quarter for the four calls), both from the run
before, as in part 5 (r7_f10_budget_during_call.py: the same split of the cost into model requests, at the same prices).
The script stops at the first try whose budget lay between those two costs and whose turn ended with
`error_max_budget_usd` while the call's `sleep` still ran (for helper-goes-on: while the helper had made fewer than four
Bash calls), at most 5 tries. Every try is a recording. Each note that lists lines is written under the driver's lock in
the same step as the list; process checks are made before that step.
"""
import sys, time
from drv import Session
from r5_lib import Procs, ask, open_tasks, results
from r6_lib import answer_to, kinds, raw, sent_at, stamps, system
from r7_lib import is_tool_result, is_tool_use
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "interrupt"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
SLEEP = {"interrupt": "53.5", "stop-task": "57.5", "helper-goes-on": "47.5"}[VARIANT]
STORY = "write a story of about 600 words about a lighthouse keeper, using no tools, and report the story"
CALLS = ("run these four Bash commands one after the other, as four separate tool calls, each after the one before has returned: "
         "`echo h1`, `echo h2`, `echo h3`, `echo h4`; then report their output")
TURN = ("Do these steps in order. 1) Use the Agent tool with run_in_background set to true to have a subagent %s. 2) Right after starting "
        "it, run the Bash command `echo go; sleep %s` in the foreground with a timeout of 120000 ms. 3) Then reply with the single word: done"
        % (CALLS if VARIANT == "helper-goes-on" else STORY, SLEEP))
SHARE = 0.25 if VARIANT == "helper-goes-on" else 0.5
PRICE = {"input_tokens": 0.1e-6, "output_tokens": 0.5e-6, "cache_read_input_tokens": 0.01e-6, "ephemeral_5m_input_tokens": 0.125e-6,
         "ephemeral_1h_input_tokens": 0.2e-6}  # claude-haiku-5-5 list prices per token, as in r7_f10
sleeps = Procs("sleep " + SLEEP)


def request_cost(u):
    cache = ((u.get("iterations") or [{}])[0].get("cache_creation")) or u.get("cache_creation") or {}
    return sum(u.get(k, 0) * PRICE[k] for k in ("input_tokens", "output_tokens", "cache_read_input_tokens")) + sum(
        cache.get(k, 0) * PRICE[k] for k in ("ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"))


def helper_bash_calls(s):
    with s.cv:
        ev = list(s.events)
    return [b["input"].get("command") for e in ev if is_tool_use("Bash", helper=True)(e) for b in e["message"]["content"] if b.get("type") == "tool_use"]


def run(label, budget=None):
    extra = ["--tools", "Bash,Agent"] + (["--max-budget-usd", "%.6f" % budget] if budget is not None else [])
    s = Session("r8-after-budget-%s-%s-%s" % (VARIANT, LABEL, label), extra=extra, hard_timeout=200, repo="r8-after-budget-%s-%s" % (VARIANT, LABEL))
    s.user(TURN)
    end = time.time() + 150
    while time.time() < end and results(s) < 1 and s.alive():
        time.sleep(0.1)
    alive_at_result = bool(sleeps.alive())
    calls_at_result = helper_bash_calls(s)
    rows = [r for r in raw(s) if r["dir"] == "recv" and isinstance(r["msg"], dict)]
    first = lambda pred: next((r for r in rows if pred(r["msg"])), None)
    use = first(is_tool_use("Bash"))
    bash_result = next((r for r in rows if use and r["t"] >= use["t"] and is_tool_result()(r["msg"])), None)
    result = first(lambda e: e.get("type") == "result")
    total = result and result["msg"].get("total_cost_usd")
    mains = [(r["t"], request_cost(r["msg"]["event"]["usage"])) for r in rows if r["msg"].get("type") == "stream_event"
             and r["msg"]["event"].get("type") == "message_delta" and not r["msg"].get("parent_tool_use_id")]
    before_call = sum(c for at, c in mains if use and at < (bash_result["t"] if bash_result else float("inf")))
    helper = (total or 0.0) - sum(c for _, c in mains)
    costs = dict(helper=round(helper, 6), when_the_bash_call_began=round(before_call, 6), with_the_helpers_requests=round(before_call + helper, 6))
    call_task = first(lambda e: e.get("type") == "system" and e.get("subtype") == "task_started" and use
                      and e.get("tool_use_id") == next(b["id"] for b in use["msg"]["message"]["content"] if b.get("type") == "tool_use"))
    note = dict(run=label, budget_usd=budget, result_subtype=result and result["msg"].get("subtype"), total_cost_usd=total, costs=costs,
                t_bash_tool_use=use and use["t"], t_bash_tool_result=bash_result and bash_result["t"], t_result=result and result["t"],
                call_task=call_task and call_task["msg"]["task_id"], sleep_alive_at_the_result=alive_at_result,
                helper_bash_calls_at_the_result=calls_at_result, list_at_the_result=open_tasks(s))
    s.note(**note)
    print({k: v for k, v in note.items() if k != "list_at_the_result"})
    stopped = budget is not None and note["result_subtype"] == "error_max_budget_usd" and s.alive()
    if stopped and VARIANT in ("interrupt", "stop-task") and alive_at_result:
        time.sleep(2.0)
        m = s.mark()
        if VARIANT == "interrupt":
            ask(s, "interrupt")
        else:
            ask(s, "stop_task", task_id=note["call_task"])
        answer = answer_to(s)
        time.sleep(5.0)
        alive = bool(sleeps.alive())
        with s.cv:
            s.note(after_the_request=dict(lines=kinds(s, m), list=open_tasks(s), sleep_alive=alive),
                   seconds_from_the_request=stamps(s, sent_at(s, "interrupt" if VARIANT == "interrupt" else "stop_task"), answer=answer,
                                                   call_notification=system("task_notification", task_id=note["call_task"])))
    elif stopped and VARIANT == "helper-goes-on":
        m = s.mark()
        time.sleep(40.0)
        alive = bool(sleeps.alive())
        with s.cv:
            later = [e for e in s.events[m:] if e.get("type") == "result"]
            ends = [e.get("status") for e in s.events[m:] if e.get("type") == "system" and e.get("subtype") == "task_notification"]
            s.note(forty_s_after_the_stop=dict(lines=kinds(s, m), helper_bash_calls_after_the_stop=helper_bash_calls(s)[len(calls_at_result):],
                                               task_notification_statuses=ends,
                                               results=[[e.get("subtype"), (e.get("origin") or {}).get("kind"), e.get("total_cost_usd")] for e in later],
                                               list=open_tasks(s), sleep_alive=alive))
    m = s.mark()

    def after_exit():
        alive = bool(sleeps.alive())
        with s.cv:
            return dict(after_the_exit=dict(lines_after_stdin_was_closed=kinds(s, m), sleep_alive=alive))

    s.close(after_exit=after_exit)
    print("killed leftovers:", sleeps.kill_left())
    return note


prev = run("calib")
for i in range(1, 6):
    c = prev["costs"]
    n = run("t%d" % i, c["when_the_bash_call_began"] + c["helper"] * SHARE)
    k, b = n["costs"], n["budget_usd"]
    good = n["result_subtype"] == "error_max_budget_usd" and k["when_the_bash_call_began"] < b < k["with_the_helpers_requests"]
    if good and (n["sleep_alive_at_the_result"] if VARIANT != "helper-goes-on" else len(n["helper_bash_calls_at_the_result"]) < 4):
        break
    if n["t_bash_tool_result"] and k["helper"] > 0:  # a run whose call ran and whose helper worked: its costs set the next budget
        prev = n
else:
    raise SystemExit("no run passed the limit as the scenario needs: record again")
