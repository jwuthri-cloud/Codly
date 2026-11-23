"""C5: `set_model` sent while an ANSWER streams (no tool call runs).

Turn 1 (on haiku) asks for a long plain answer (the numbers from 1 to 400). When the first text delta of the answer
has come, the host sends `set_model sonnet`. The note of the turn says which model the lines before and after the
answer to `set_model` name (`message_start` of the stream, the `assistant` lines, `result.modelUsage`) and how many
text deltas came after it (so that "the answer was still streaming" is recorded, not assumed).
Turn 2 is one tiny turn on the new model; then `set_model haiku` and turn 3.

argv[1]: a label for the recording (1, 2).
"""
import sys
from drv import Session
from r5_lib import ask, next_turn, results
from r6_lib import answer_to
run = sys.argv[1] if len(sys.argv) > 1 else "1"
s = Session("r6-model-while-streaming-" + run, extra=["--tools", "Bash"], hard_timeout=200)
mark = 0


def text_delta(e):
    return e.get("type") == "stream_event" and e.get("event", {}).get("delta", {}).get("type") == "text_delta"


def models(label, split=None):
    """What the lines of this turn say about the model; `split`: the event index of the answer to set_model."""
    global mark
    with s.cv:
        ev, start, mark = s.events[mark:], mark, len(s.events)

    def part(lines):
        return dict(stream_message_start_model=[e["event"]["message"].get("model") for e in lines if e.get("type") == "stream_event" and e["event"].get("type") == "message_start"],
                    assistant_message_model=[e["message"].get("model") for e in lines if e.get("type") == "assistant"],
                    text_deltas=sum(1 for e in lines if text_delta(e)))
    note = dict(turn=label, init_model=[e.get("model") for e in ev if e.get("type") == "system" and e.get("subtype") == "init"],
                result_modelUsage_keys=[sorted((e.get("modelUsage") or {}).keys()) for e in ev if e.get("type") == "result"],
                result_model_usage_since_last_result_keys=[sorted((e.get("model_usage_since_last_result") or {}).keys()) for e in ev if e.get("type") == "result"])
    if split is None:
        note.update(part(ev))
    else:
        note.update(before_the_set_model_answer=part(ev[:split - start]), after_the_set_model_answer=part(ev[split - start:]))
    s.note(**note)
    print(label, note)


def applied():
    return ask(s, "get_settings").get("response", {}).get("applied", {}).get("model")


s.control("initialize", hooks=None)
s.user("Write the numbers from 1 to 400, one per line, and nothing else.")
print("the answer streams:", bool(s.wait_for(text_delta, 40)))
ask(s, "set_model", model="sonnet")
with s.cv:
    split = next(i for i, e in enumerate(s.events) if answer_to(s)(e)) + 1
print("applied.model (while the answer streams):", applied())
print("result 1:", s.wait_results(1, 90))
models("turn 1", split)
print("turn 2:", next_turn(s, "Reply with the single word: two"))
models("turn 2")
ask(s, "set_model", model="haiku")
print("turn 3:", next_turn(s, "Reply with the single word: three"))
models("turn 3")
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
