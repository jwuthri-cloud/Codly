"""C8: which model the lines name after `set_model`, and `system/init.model` against `assistant.message.model`.

argv[1]: midturn         started on haiku; `set_model sonnet` while a tool call of turn 1 runs; turn 2; then
                         `set_model default` and turn 3 (ONE tiny turn on the account's default model); then
                         `set_model haiku` and turn 4
         start-alias     started with --model haiku
         start-full      started with --model claude-haiku-5-5
         start-dated     started with --model claude-haiku-4-5-20251001
         start-undated   started with --model claude-haiku-4-5
The start-* runs are one tiny turn each.
"""
import sys, time
from drv import Session
from r5_lib import ask, next_turn, results
variant = sys.argv[1] if len(sys.argv) > 1 else "midturn"
start = {"midturn": "haiku", "start-alias": "haiku", "start-full": "claude-haiku-5-5", "start-dated": "claude-haiku-4-5-20251001",
         "start-undated": "claude-haiku-4-5"}[variant]
s = Session("r5-model-" + variant, extra=["--tools", "Bash"], model=start, hard_timeout=200)
mark = 0

def models(label):
    """What this turn's lines say about the model."""
    global mark
    with s.cv:
        ev, mark = s.events[mark:], len(s.events)
    init = [e.get("model") for e in ev if e.get("type") == "system" and e.get("subtype") == "init"]
    assistant = [e["message"].get("model") for e in ev if e.get("type") == "assistant"]
    usage = [sorted((e.get("modelUsage") or {}).keys()) for e in ev if e.get("type") == "result"]
    since = [sorted((e.get("model_usage_since_last_result") or {}).keys()) for e in ev if e.get("type") == "result"]
    s.note(turn=label, init_model=init, assistant_message_model=assistant, result_modelUsage_keys=usage,
           result_model_usage_since_last_result_keys=since)
    print("%-10s init=%s assistant=%s modelUsage=%s since_last=%s" % (label, init, assistant, usage, since))

def applied():
    return ask(s, "get_settings").get("response", {}).get("applied", {}).get("model")

s.control("initialize", hooks=None)
print("applied.model at start:", applied())
if variant != "midturn":
    print("turn:", next_turn(s, "Reply with the single word: pong"))
    models("turn 1")
    print("applied.model:", applied())
    print("cost:", s.cost(), "exit:", s.close())
    sys.exit(0)

s.user("Run the Bash command `echo a; sleep 8; echo b` and then reply with the single word: one")
print("tool started:", bool(s.wait_tool_use(name="Bash")))
time.sleep(2.0)
ask(s, "set_model", model="sonnet")
print("applied.model (mid-turn):", applied())
print("result 1:", s.wait_results(1, 60))
models("turn 1")
print("turn 2:", next_turn(s, "Reply with the single word: two"))
models("turn 2")
ask(s, "set_model", model="default")
print("applied.model:", applied())
print("turn 3:", next_turn(s, "Reply with the single word: three"))
models("turn 3")
ask(s, "set_model", model="haiku")
print("turn 4:", next_turn(s, "Reply with the single word: four"))
models("turn 4")
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
