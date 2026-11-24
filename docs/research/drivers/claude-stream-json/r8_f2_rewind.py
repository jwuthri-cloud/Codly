"""F2: rewind a conversation with `--resume <id> --resume-session-at <uuid>`.

Process a (r8-rewind-<label>-a): a new session with `--replay-user-messages`; three turns, each tells one secret word
           (WORDS) and asks for "noted". Each user message carries a uuid of the host. A note per turn: the `uuid` of
           every assistant line of the main thread, the uuid of the replayed user line, the `user_message_uuid` of
           the result.
Process b (…-b): `--resume <a's session> --resume-session-at <uuid of turn 1's LAST assistant line>`: one turn asks
           for every secret word told so far. Note: which words the answer names, the session id of init and result.
Process c (…-c): `--resume <a's session>` with no `--resume-session-at`: the same question (which part does a plain
           resume continue after b?).
Process d (…-d): `--resume <a's session> --resume-session-at <uuid of turn 3's last assistant line>`: the same question
           (can the dropped part be resumed again?).
After each process a note counts, in the session's transcript file, its lines and the lines that name each word
(r8_lib.transcript: counts only).

argv[1]: a label for the recording (1, 2): the whole scenario, recorded more than once.
"""
import sys, uuid
from drv import Session
from r5_lib import next_turn
from r8_lib import assistant_uuids, last_result, transcript, words_in
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
NAME = "r8-rewind-" + LABEL
WORDS = ("lantern", "pebble", "violet")
ASK = "List every secret word I asked you to remember in this conversation, in one line. If there is none, reply with the single word: none"


def start(part, extra=(), new=False):
    return Session("%s-%s" % (NAME, part), extra=["--tools", "Bash"] + list(extra), session_id="new" if new else None, hard_timeout=150, repo=NAME)


def ask_words(s, part, sid):
    m = s.mark()
    ok = next_turn(s, ASK, 60)
    init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 1, m) or {}
    r = last_result(s, m)
    s.note(asked=part, ok=ok, words_named=words_in(r.get("result"), WORDS), init_session_id_is_the_first_session=init.get("session_id") == sid,
           result_session_id_is_the_first_session=r.get("session_id") == sid)
    print(part, "words:", words_in(r.get("result"), WORDS), "same session:", r.get("session_id") == sid)
    return r


a = start("a", ["--replay-user-messages"], new=True)
sid = a.sid
ends = []
for i, w in enumerate(WORDS, 1):
    m, host_uuid = a.mark(), str(uuid.uuid4())
    ok = next_turn(a, "Remember this secret word: %s. Reply with the single word: noted" % w, 60, uuid=host_uuid)
    with a.cv:
        replay = [e for e in a.events[m:] if e.get("type") == "user" and not e.get("parent_tool_use_id") and isinstance(e["message"].get("content"), str)]
    r = last_result(a, m)
    uu = assistant_uuids(a, m)
    ends.append(uu[-1] if uu else None)
    a.note(turn=i, ok=ok, assistant_uuids=uu, replayed_user_lines=[{"uuid_is_the_hosts": e.get("uuid") == host_uuid, "isReplay": e.get("isReplay")} for e in replay],
           result_user_message_uuid_is_the_hosts=r.get("user_message_uuid") == host_uuid)
a.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))
print("turn ends:", ends)
if None in ends:
    raise SystemExit("a turn had no assistant line: record again")
b = start("b", ["--resume", sid, "--resume-session-at", ends[0]])
b.note(resume_session_at="the uuid of the last assistant line of turn 1 of process a")
ask_words(b, "b", sid)
b.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))
c = start("c", ["--resume", sid])
ask_words(c, "c", sid)
c.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))
d = start("d", ["--resume", sid, "--resume-session-at", ends[2]])
d.note(resume_session_at="the uuid of the last assistant line of turn 3 of process a")
ask_words(d, "d", sid)
d.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS)))
