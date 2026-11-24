"""F3: fork a conversation with `--resume <id> --fork-session`, alone and with `--resume-session-at`.

Process a (r8-fork-<label>-a): a new session; two turns, each tells one secret word (WORDS) and asks for "noted". A note
           per turn: the `uuid` of every assistant line of the main thread, `total_cost_usd`.
Process b (…-b): `--resume <a's session> --fork-session`: one turn asks for every secret word told so far.
Process c (…-c): `--resume <a's session> --fork-session --resume-session-at <uuid of turn 1's last assistant line>`: the
           same question.
Process d (…-d): `--resume <a's session>` (no fork): the same question (does the old session answer as before?).
The note of each later process: which words the answer names; whether the session id of its init and of its result
is a's; its result's `total_cost_usd` next to a's last one and to the cost of its own turn
(`model_usage_since_last_result`: costUSD summed). After each process a note counts the lines of a's transcript and
the session files of its folder (r8_lib.transcript: counts only).

argv[1]: a label for the recording (1, 2): the whole scenario, recorded more than once.
"""
import sys
from drv import Session
from r5_lib import next_turn
from r8_lib import assistant_uuids, last_result, transcript, words_in
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
NAME = "r8-fork-" + LABEL
WORDS = ("lantern", "pebble")
ASK = "List every secret word I asked you to remember in this conversation, in one line. If there is none, reply with the single word: none"


def start(part, extra=(), new=False):
    return Session("%s-%s" % (NAME, part), extra=["--tools", "Bash"] + list(extra), session_id="new" if new else None, hard_timeout=150, repo=NAME)


def turn_cost(r):
    return round(sum(m.get("costUSD", 0.0) for m in (r.get("model_usage_since_last_result") or {}).values()), 8)


a = start("a", new=True)
sid = a.sid
ends = []
for i, w in enumerate(WORDS, 1):
    m = a.mark()
    ok = next_turn(a, "Remember this secret word: %s. Reply with the single word: noted" % w, 60)
    uu = assistant_uuids(a, m)
    ends.append(uu[-1] if uu else None)
    a.note(turn=i, ok=ok, assistant_uuids=uu, total_cost_usd=last_result(a, m).get("total_cost_usd"))
a_cost = a.cost()
a.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))
if None in ends:
    raise SystemExit("a turn had no assistant line: record again")


def ask_words(part, extra, what):
    s = start(part, extra)
    if what:
        s.note(resume_session_at=what)
    m = s.mark()
    ok = next_turn(s, ASK, 60)
    init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 1, m) or {}
    r = last_result(s, m)
    s.note(asked=part, ok=ok, words_named=words_in(r.get("result"), WORDS), init_session_id_is_the_first_session=init.get("session_id") == sid,
           result_session_id_is_the_first_session=r.get("session_id") == sid, total_cost_usd=r.get("total_cost_usd"),
           first_sessions_last_total_cost_usd=a_cost, cost_of_this_turn=turn_cost(r))
    print(part, "words:", words_in(r.get("result"), WORDS), "same session:", r.get("session_id") == sid, "cost:", r.get("total_cost_usd"), "a:", a_cost, "turn:", turn_cost(r))
    s.close(after_exit=lambda: dict(transcript_of_the_first_session=transcript(sid, WORDS)))


ask_words("b", ["--resume", sid, "--fork-session"], None)
ask_words("c", ["--resume", sid, "--fork-session", "--resume-session-at", ends[0]], "the uuid of the last assistant line of turn 1 of process a")
ask_words("d", ["--resume", sid], None)
