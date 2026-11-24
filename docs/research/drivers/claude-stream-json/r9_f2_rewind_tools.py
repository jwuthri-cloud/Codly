"""F2: rewind a conversation that has tool calls, a helper and an interrupted turn (`--resume <id>
--resume-session-at <uuid>`), and the hidden flag `--resume-drops-turn`.

Session 1 (r9-rewind-tools-<label>-a … -f), `--tools Bash,Read,Edit,Agent`, mode acceptEdits, every request allowed.
Process a: a new session with `--replay-user-messages` (and `--include-partial-messages`, as every driver run and as
           Codly passes it). Four turns, each tells one secret word (WORDS) and carries a uuid of the host:
           turn 1 text only; turn 2 an Edit of hello.txt and a Bash call; turn 3 a helper (Agent) that runs a Bash
           call; turn 4 a Bash call `echo copper-start; sleep 19.5; echo copper-end`, and an `interrupt` once its
           `sleep` runs. A note per turn: the `uuid` of every assistant line of the main thread, whether each is in
           the session's transcript, how many stream_event lines carry a uuid and how many of those are in it, the
           replayed user line, the result's `user_message_uuid`.
Then one process each, every one asks for every secret word told so far (no tools), in this order:
  b  `--resume-session-at <the last assistant line of turn 4>` (the interrupted tool call's line)
  c  `--resume-session-at <the last assistant line of turn 3>`
  d  `--resume-session-at <the last assistant line of turn 2>`
  e  `--resume-session-at <the uuid of turn 2's USER message>` (the host's uuid)
  f  `--resume-session-at <the last assistant line of turn 3>` again: the same uuid as c, now that d and e added
     turns on an earlier branch (c is the control: the same uuid before those turns were added)
After each process: counts from the transcript (r8_lib.transcript) and the line types it got (r9_lib.transcript_types:
the CLI's own type words, nothing of the conversation).

Session 2 (r9-rewind-drops-<label>-a … -c): three text turns (lantern, pebble, violet), host uuids U1 … U3.
  b  `--resume-session-at <end of turn 1> --resume-drops-turn <U3>` (the dropped part also holds turn 2)
  c  `--resume-session-at <end of turn 2> --resume-drops-turn <U3>` (the dropped part is turn 3 alone)

argv[1]: a label for the recording (1, 2): the whole scenario, recorded more than once.
"""
import sys, time, uuid
from drv import Session
from r5_lib import Procs, next_turn
from r6_lib import kinds, wait_alive
from r8_lib import assistant_uuids, last_result, transcript, words_in
from r9_lib import transcript_types, transcript_uuids
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
WORDS = ("lantern", "pebble", "violet", "copper")
ASK = ("Without using any tool: list every secret word I asked you to remember in this conversation, in one line. "
       "If there is none, reply with the single word: none")
TURNS = (
    "Remember this secret word: lantern. Reply with the single word: noted",
    "Remember this secret word: pebble. Read hello.txt, then use the Edit tool to change its text from `hello` to `hello pebble`, "
    "then run the Bash command `echo pebble-ok`. Then reply with the single word: done",
    "Remember this secret word: violet. Use the Agent tool once: ask a helper to run the Bash command `echo violet-helper` and to "
    "report its output. Then reply with the single word: done",
    "Remember this secret word: copper. Run the Bash command `echo copper-start; sleep 19.5; echo copper-end` in the foreground "
    "(not in the background), then reply with the single word: done",
)


def transcript_len(sid):
    t = transcript(sid, WORDS)
    return t.get("lines", 0)


def turn_note(s, i, m, host_uuid, sid):
    with s.cv:
        ev = s.events[m:]
    replay = [e for e in ev if e.get("type") == "user" and not e.get("parent_tool_use_id") and isinstance(e["message"].get("content"), str)]
    streamed = [e.get("uuid") for e in ev if e.get("type") == "stream_event" and e.get("uuid")]
    known = transcript_uuids(sid)
    uu = assistant_uuids(s, m)
    r = last_result(s, m)
    s.note(turn=i, assistant_uuids=uu, assistant_uuids_in_the_transcript=[u in known for u in uu],
           stream_events_with_a_uuid=len(streamed), stream_event_uuids_in_the_transcript=sum(1 for u in streamed if u in known),
           replayed_user_lines=[{"uuid_is_the_hosts": e.get("uuid") == host_uuid, "isReplay": e.get("isReplay")} for e in replay],
           result_subtype=r.get("subtype"), result_user_message_uuid_is_the_hosts=r.get("user_message_uuid") == host_uuid,
           tools_used=[b.get("name") for e in ev if e.get("type") == "assistant" for b in e["message"]["content"] if b.get("type") == "tool_use"])
    return uu


def ask_words(name, part, sid, at, why, extra=()):
    before = transcript_len(sid)
    s = Session("%s-%s" % (name, part), repo=name, extra=["--tools", "Bash,Read,Edit,Agent", "--resume", sid, "--resume-session-at", at] + list(extra),
                session_id=None, hard_timeout=120)
    s.note(resume_session_at=why)
    ok = next_turn(s, ASK, 60)
    time.sleep(0.5)
    r = last_result(s)
    init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 1) or {}
    with s.cv:
        used = [b.get("name") for e in s.events if e.get("type") == "assistant" for b in e["message"]["content"] if b.get("type") == "tool_use"]
    s.note(asked=part, ok=ok, words_named=words_in(r.get("result"), WORDS), result=(r.get("result") or "")[:300], tools_used=used,result_subtype=r.get("subtype"), result_errors=r.get("errors"),
           init_came=bool(init), init_session_id_is_the_first_session=init.get("session_id") == sid if init else None,
           result_session_id_is_the_first_session=r.get("session_id") == sid, lines=kinds(s))
    print(name, part, "words:", words_in(r.get("result"), WORDS), r.get("subtype"), r.get("errors"))
    s.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS), transcript_types_added=transcript_types(sid, before)))
    return r


# ---- session 1
NAME = "r9-rewind-tools-" + LABEL
a = Session(NAME + "-a", repo=NAME, extra=["--tools", "Bash,Read,Edit,Agent", "--replay-user-messages"], hard_timeout=300)
with open(a.cwd + "/hello.txt", "w") as f:
    f.write("hello\n")
sid, ends, users = a.sid, [], []
sleeper = Procs("sleep 19.5")
for i, text in enumerate(TURNS, 1):
    m, host_uuid = a.mark(), str(uuid.uuid4())
    users.append(host_uuid)
    if i < 4:
        ok = next_turn(a, text, 120, uuid=host_uuid)
        a.note(turn_ok=ok)
    else:
        a.user(text, uuid=host_uuid)
        alive = wait_alive(sleeper, 60)
        time.sleep(1.0)
        a.note(before_the_interrupt=True, sleep_19_5_alive=alive)
        a.control("interrupt")
        a.note(turn_ok=a.wait_results(4, 60))
    time.sleep(0.5)
    uu = turn_note(a, i, m, host_uuid, sid)
    ends.append(uu[-1] if uu else None)
a.close(after_exit=lambda: dict(transcript=transcript(sid, WORDS), left=sleeper.kill_left()))
print("turn ends:", ends)
if None in ends:
    raise SystemExit("a turn had no assistant line: record again")
ask_words(NAME, "b", sid, ends[3], "the uuid of the last assistant line of turn 4 (the interrupted tool call) of process a")
ask_words(NAME, "c", sid, ends[2], "the uuid of the last assistant line of turn 3 of process a")
ask_words(NAME, "d", sid, ends[1], "the uuid of the last assistant line of turn 2 of process a")
ask_words(NAME, "e", sid, users[1], "the uuid of turn 2's user message (the host's uuid) of process a")
ask_words(NAME, "f", sid, ends[2], "the uuid of the last assistant line of turn 3 of process a, again (as c)")

# ---- session 2
NAME2 = "r9-rewind-drops-" + LABEL
a = Session(NAME2 + "-a", repo=NAME2, extra=["--tools", "Bash"], hard_timeout=150)
sid2, ends2, users2 = a.sid, [], []
for w in WORDS[:3]:
    m, host_uuid = a.mark(), str(uuid.uuid4())
    users2.append(host_uuid)
    ok = next_turn(a, "Remember this secret word: %s. Reply with the single word: noted" % w, 60, uuid=host_uuid)
    uu = assistant_uuids(a, m)
    ends2.append(uu[-1] if uu else None)
    a.note(turn=w, ok=ok, assistant_uuids=uu, result_user_message_uuid_is_the_hosts=last_result(a, m).get("user_message_uuid") == host_uuid)
a.close(after_exit=lambda: dict(transcript=transcript(sid2, WORDS)))
if None in ends2:
    raise SystemExit("a turn had no assistant line: record again")
ask_words(NAME2, "b", sid2, ends2[0], "the end of turn 1, and --resume-drops-turn <U3>", ["--resume-drops-turn", users2[2]])
ask_words(NAME2, "c", sid2, ends2[1], "the end of turn 2, and --resume-drops-turn <U3>", ["--resume-drops-turn", users2[2]])
