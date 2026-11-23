"""F10: `--max-budget-usd` reached while a long tool call runs (`sleep 61.5`), and while a helper works: is the call
cut off or does it run to its end; what happens to the helper; what does the stop say?

The cost only grows when a model request ends, and a running Bash call makes none. So the limit can only be passed
DURING the call by a request of a helper. One turn: the model starts a helper in the background that writes a story
of about 600 words with no tool (one long model request), then runs `echo go; sleep 61.5` in the foreground, then
answers. Every run is a new session in the same folder.

Where the money goes, per run (a note, `costs`): each model request of the MAIN thread costs what its final usage
(the `message_delta` stream event) says at the list prices of claude-haiku-5-5 (PRICE below: per token; they give the
`costUSD` of every `model_usage_since_last_result` of these recordings to the last digit). The CLI streams no events
of a helper, so the helper's cost is the rest: `total_cost_usd` minus the main requests. From that: the cost when the
Bash call began (main requests before its `tool_use`) and the cost once the helper had written its story.

Run calib (r7-budget-during-call-<label>-calib): no budget.
Runs t1, t2, … (…-t1 …): `--max-budget-usd` = the cost when the Bash call began + half the helper's cost, both from
       the run before (the cache moves the cost of a request from run to run). The script stops at the first run in
       which the budget lay between those two costs, the story came while the call ran, and the turn ended with
       `error_max_budget_usd` (at most 5 tries). Every try is a recording.
Each run's note gives, as time stamps of the lines: the main Bash call's `tool_use` and tool result, the helper's story
line, the `result`; whether the `sleep 61.5` process was alive at the result; the task lines; the list. With a budget
the host then reads 3 s (note: lines, list, the `sleep`), sends one more message (a process whose budget is spent:
part 4, section 10), and closes stdin; the last note has the lines after the close and the `sleep`.
The story came "while the call ran" when its line is after the call's `tool_use` and before the call's result (or
before the turn's `result` when the call got none).

argv[1]: a label for the recording (1, 2): the whole scenario, recorded more than once.
"""
import json, sys, time
from drv import Session
from r5_lib import Procs, next_turn, open_tasks, results
from r6_lib import kinds, raw
from r7_lib import is_tool_result, is_tool_use
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
TURN = ("Do these steps in order. 1) Use the Agent tool with run_in_background set to true to have a subagent write a story of about "
        "600 words about a lighthouse keeper, using no tools, and report the story. 2) Right after starting it, run the Bash command "
        "`echo go; sleep 61.5` in the foreground with a timeout of 120000 ms. 3) Then reply with the single word: done")
PRICE = {"input_tokens": 0.1e-6, "output_tokens": 0.5e-6, "cache_read_input_tokens": 0.01e-6, "ephemeral_5m_input_tokens": 0.125e-6,
         "ephemeral_1h_input_tokens": 0.2e-6}
sleeps = Procs("[s]leep 61.5")


def request_cost(u):
    cache = ((u.get("iterations") or [{}])[0].get("cache_creation")) or u.get("cache_creation") or {}
    return sum(u.get(k, 0) * PRICE[k] for k in ("input_tokens", "output_tokens", "cache_read_input_tokens")) + sum(
        cache.get(k, 0) * PRICE[k] for k in ("ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"))


def run(label, budget=None):
    extra = ["--tools", "Bash,Agent"] + (["--max-budget-usd", "%.6f" % budget] if budget is not None else [])
    s = Session("r7-budget-during-call-%s-%s" % (LABEL, label), extra=extra, hard_timeout=200, repo="r7-budget-during-call-" + LABEL)
    s.user(TURN)
    end = time.time() + 150
    while time.time() < end and results(s) < 1 and s.alive():
        time.sleep(0.1)
    alive_at_result = bool(sleeps.alive())
    rows = [r for r in raw(s) if r["dir"] == "recv" and isinstance(r["msg"], dict)]
    first = lambda pred: next((r for r in rows if pred(r["msg"])), None)
    t = lambda r: r and r["t"]
    use = first(is_tool_use("Bash"))
    bash_result = next((r for r in rows if use and r["t"] >= use["t"] and is_tool_result()(r["msg"])), None)
    story = first(lambda e: e.get("type") == "assistant" and e.get("parent_tool_use_id") and any(b.get("type") == "text" for b in e["message"]["content"]))
    result = first(lambda e: e.get("type") == "result")
    total = result and result["msg"].get("total_cost_usd")
    mains = [(r["t"], request_cost(r["msg"]["event"]["usage"])) for r in rows if r["msg"].get("type") == "stream_event"
             and r["msg"]["event"].get("type") == "message_delta" and not r["msg"].get("parent_tool_use_id")]
    # The request that asks for the Bash call ends (its message_delta) a moment after the call's tool_use line; no main
    # request ends while the call runs. So every main request that ended before the call's result was paid before it.
    before_call = sum(c for at, c in mains if use and at < (bash_result["t"] if bash_result else float("inf")))
    helper = (total or 0.0) - sum(c for _, c in mains)
    costs = dict(main_requests=[[round(at, 3), round(c, 6)] for at, c in mains], helper=round(helper, 6), when_the_bash_call_began=round(before_call, 6),
                 once_the_story_was_written=round(before_call + helper, 6))
    task_lines = [{k: r["msg"].get(k) for k in ("subtype", "status", "patch")} | {"_t": r["t"]} for r in rows
                  if r["msg"].get("type") == "system" and r["msg"].get("subtype") in ("task_updated", "task_notification")]
    note = dict(run=label, budget_usd=budget, result_subtype=result and result["msg"].get("subtype"), result_errors=result and result["msg"].get("errors"),
                total_cost_usd=total, costs=costs, t_bash_tool_use=t(use), t_bash_tool_result=t(bash_result), t_helper_story=t(story), t_result=t(result),
                sleep_61_5_alive_at_the_result=alive_at_result,
                main_assistant_lines_after_the_bash_result=sum(1 for r in rows if bash_result and r["t"] > bash_result["t"] and r["msg"].get("type") == "assistant"
                                                               and not r["msg"].get("parent_tool_use_id")),
                task_lines=task_lines, list_at_the_result=open_tasks(s))
    s.note(**note)
    print(json.dumps({k: v for k, v in note.items() if k != "task_lines"})[:1200])
    if budget is not None and s.alive():
        m = s.mark()
        time.sleep(3.0)
        s.note(three_s_after_the_result=dict(lines=kinds(s, m), list=open_tasks(s), sleep_61_5_alive=bool(sleeps.alive())))
        m = s.mark()
        second = next_turn(s, "Reply with the single word: alive", 30)
        r2 = [e for e in s.events[m:] if e.get("type") == "result"]
        s.note(second_turn=dict(result_came=second, result_subtype=r2 and r2[-1].get("subtype"), list=open_tasks(s), sleep_61_5_alive=bool(sleeps.alive())))
    m = s.mark()
    s.close(after_exit=lambda: dict(after_the_exit=dict(lines_after_stdin_was_closed=kinds(s, m), sleep_61_5_alive=bool(sleeps.alive()))))
    print("killed leftovers:", sleeps.kill_left())
    return note


prev = run("calib")
for i in range(1, 6):
    c = prev["costs"]
    n = run("t%d" % i, c["when_the_bash_call_began"] + c["helper"] / 2)
    k, b = n["costs"], n["budget_usd"]
    during = n["t_helper_story"] and n["t_bash_tool_use"] and n["t_bash_tool_use"] < n["t_helper_story"] < (n["t_bash_tool_result"] or n["t_result"])
    if n["result_subtype"] == "error_max_budget_usd" and during and k["when_the_bash_call_began"] < b < k["once_the_story_was_written"]:
        break
    if n["t_bash_tool_result"] and k["helper"] > 0:  # a run whose call ran and whose helper worked: its costs set the next budget
        prev = n
else:
    raise SystemExit("no run passed the limit while the call ran: record again")
