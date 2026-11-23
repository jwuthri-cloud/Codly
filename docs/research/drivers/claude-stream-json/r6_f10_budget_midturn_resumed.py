"""F10: `--max-budget-usd` on a RESUMED process, reached in the middle of a turn, after a tool call; and a second
turn after it.

Every turn asks for three tool calls in a row (`echo one`, `echo two`, `echo three`) and a one-word answer: four
model requests.

Run a (r6-budget-midturn-<label>-a): a new session, no budget, two turns.
Run b (r6-budget-midturn-<label>-b): `--resume`, no budget, one turn. T = what this turn alone cost.
Run c1, c2, … (r6-budget-midturn-<label>-c1 …): `--resume` with `--max-budget-usd` = a part of T (0.5, then 0.7, then
       0.85), one turn. The script stops at the first run in which the limit came after at least one tool call had run.
       After the error result the host waits 2 s and sends one more message.
The cache makes the cost of a request move, so the part of T that works is found by trying: every try is a recording.

argv[1]: a label for the recording (1, 2): the whole scenario, recorded more than once.
"""
import json
import sys
import time
from drv import Session
from r5_lib import next_turn, results
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
TURN = ("Run the Bash command `echo one`. After you see its output, run `echo two` in a second, separate tool call. After that, "
        "run `echo three` in a third, separate tool call. Then reply with the single word: done")


def last_result(s):
    with s.cv:
        r = [e for e in s.events if e.get("type") == "result"]
    return r[-1] if r else {}


def alone(r):
    return sum(v.get("costUSD") or 0.0 for v in (r.get("model_usage_since_last_result") or {}).values())


def tool_results(s, start=0):
    with s.cv:
        return sum(1 for e in s.events[start:] if e.get("type") == "user" and isinstance(e["message"]["content"], list)
                   for b in e["message"]["content"] if b.get("type") == "tool_result")


def run(label, sid=None, budget=None, turns=1):
    extra = ["--tools", "Bash"]
    if sid:
        extra += ["--resume", sid]
    if budget is not None:
        extra += ["--max-budget-usd", "%.6f" % budget]
    s = Session("r6-budget-midturn-%s-%s" % (LABEL, label), extra=extra, session_id=None if sid else "new", hard_timeout=150, repo="r6-budget-midturn-" + LABEL)
    for _ in range(turns):
        ok = next_turn(s, TURN)
    r = last_result(s)
    ran = tool_results(s)
    note = dict(run=label, budget_usd=budget, turn_ok=ok, result_subtype=r.get("subtype"), errors=r.get("errors"), total_cost_usd=r.get("total_cost_usd"),
                cost_of_the_last_turn_alone=alone(r), tool_calls_that_ran_in_the_last_turn=ran if turns == 1 else None)
    if budget is not None:
        time.sleep(2.0)
        m, n = s.mark(), results(s)
        alive = s.alive()
        second = next_turn(s, "Reply with the single word: alive", 30) if alive else False
        r2 = last_result(s) if results(s) > n else {}
        with s.cv:
            requests = sum(1 for e in s.events[m:] if e.get("type") == "assistant")
        note.update(alive_2s_after_the_limit=alive, second_turn=dict(result_came=second, result_subtype=r2.get("subtype"), result=r2.get("result"), errors=r2.get("errors"),
                                                                     total_cost_usd=r2.get("total_cost_usd"), assistant_lines=requests, alive_after_it=s.alive()))
    s.note(**note)
    print(json.dumps(note), "exit:", s.close())
    return sid or s.sid, alone(r), r.get("subtype"), ran


sid, _, _, _ = run("a", turns=2)
_, t, _, _ = run("b", sid)
for i, part in enumerate((0.5, 0.7, 0.85), 1):
    _, _, subtype, ran = run("c%d" % i, sid, part * t)
    if subtype == "error_max_budget_usd" and ran >= 1:
        break
else:
    raise SystemExit("no run reached the limit after a tool call: record again")
