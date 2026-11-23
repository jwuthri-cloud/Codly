"""F14: does `--max-budget-usd` count the cost of the session before a `--resume`?

Every turn asks for one tool call (`echo one`) and a one-word answer.

Run a (r5-budget-resume-a): a new session, no budget, five turns.
Run b  (r5-budget-resume-b): `--resume` of it, no budget, one turn. T = what this turn alone cost
       (`model_usage_since_last_result`); `total_cost_usd` shows whether the counter goes on from run a.
Run c  (r5-budget-resume-c): `--resume` with `--max-budget-usd` 2 x T: above one turn, far below what the session has
       cost so far. One turn: success means the cost from before the resume is not counted.
Run d  (r5-budget-resume-d): control: `--resume` with a budget of 0.00002 USD, below one model request: the limit must hit.
"""
from drv import Session
from r5_lib import next_turn

def run(label, sid=None, budget=None, turns=1):
    extra = ["--tools", "Bash"]
    if sid:
        extra += ["--resume", sid]
    if budget is not None:
        extra += ["--max-budget-usd", "%.6f" % budget]
    s = Session("r5-budget-resume-" + label, extra=extra, session_id=None if sid else "new", hard_timeout=150, repo="r5-budget-resume")
    for _ in range(turns):
        ok = next_turn(s, "Run the Bash command `echo one` and then reply with the single word: done")
    with s.cv:
        r = [e for e in s.events if e.get("type") == "result"]
    last = r[-1] if r else {}
    turn_cost = sum(v.get("costUSD") or 0.0 for v in (last.get("model_usage_since_last_result") or {}).values())
    s.note(run=label, budget_usd=budget, last_turn_ok=ok, result_subtype=last.get("subtype"), total_cost_usd=last.get("total_cost_usd"),
           cost_of_the_last_turn_alone=turn_cost, session_id_is_the_resumed_one=(last.get("session_id") == sid) if sid else None)
    print("run %s: budget=%s subtype=%s total_cost_usd=%s last turn=%.6f errors=%s same session id=%s exit=%s" % (
        label, budget, last.get("subtype"), last.get("total_cost_usd"), turn_cost, last.get("errors"),
        (last.get("session_id") == sid) if sid else None, s.close()))
    return sid or s.sid, last.get("total_cost_usd") or 0.0, turn_cost

sid, total_a, _ = run("a", turns=5)
_, total_b, t = run("b", sid)
x = 2 * t
if not 1.5 * x < total_b:
    raise SystemExit("the budget (%.6f) is not far below the session's cost so far (%.6f): record again" % (x, total_b))
run("c", sid, x)
run("d", sid, 0.00002)
