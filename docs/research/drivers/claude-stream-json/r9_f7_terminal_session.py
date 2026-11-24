"""F7: a conversation started in the terminal. Where a folder's sessions are kept, whether `--resume <id>` in
stream-json picks up a session that the interactive `claude` started, and what `claude --resume <unknown id>` says in
a terminal.

Process a (r9-terminal-<label>-a: notes only, no stream): the interactive `claude --model haiku --strict-mcp-config
           --setting-sources local` in a pseudo-terminal (r9_lib.Terminal), in the scenario's work folder. If it asks
           whether you trust the folder, the script picks "Yes, I trust this folder" (the CLI saves that for this work
           folder, in the HOME's `.claude.json`, as for any folder you trust). It types one message ("remember
           the word meadow", reply "noted"), waits until the session's transcript has an assistant line, types
           `/exit`. The screen is read only to wait for words: it is never written to a recording.
           A note: the folder of ~/.claude/projects that holds the work folder's sessions (its name, made from the
           path), the session files in it (names and counts), and the session's transcript as line types and its
           `entrypoint` (r9_lib.transcript_types: the CLI's own words, nothing of the conversation).
Process b (…-b): `--resume <that session>` in stream-json (the driver's usual flags): one turn asks for the secret
           word. A note: whether init and the result carry the terminal session's id, the words named.
Process c (…-c: notes only): the interactive `claude --resume <a uuid no session has>` in a pseudo-terminal: the text
           it prints (escape codes taken out, cut to 300 characters) and its exit code.

This script runs with the owner's REAL HOME and login, and answering the trust question writes a lasting entry
`projects["<CAPTURE_DIR>/work/r9-terminal-<label>"]` into the real `~/.claude.json`: get the owner's OK before you
record again, and tell them which entries to remove afterwards.

argv[1]: a label for the recording (1, 2).
"""
import glob, json, os, sys, time, uuid
from drv import OUT, Session, make_repo
from r5_lib import next_turn
from r8_lib import last_result, words_in
from r9_lib import Terminal, plain, session_dir, transcript_types
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
NAME = "r9-terminal-" + LABEL
ARGS = ["--model", "haiku", "--strict-mcp-config", "--setting-sources", "local"]
cwd = make_repo(NAME)


def notes_only(part):
    """A recording with notes and no CLI stream (the terminal runs): the same raw format as the driver's."""
    path = os.path.join(OUT, "raw", "%s-%s.ndjson" % (NAME, part))
    f = open(path, "w")
    t0 = time.time()

    def note(**kv):
        f.write(json.dumps({"t": round(time.time() - t0, 3), "dir": "meta", "msg": kv}) + "\n")
        f.flush()
    return f, note


def sessions():
    d = session_dir(cwd)
    return sorted(os.path.basename(p) for p in glob.glob(os.path.join(d, "*.jsonl"))) if d else []


def entrypoint(sid):
    d = session_dir(cwd)
    for line in open(os.path.join(d, sid + ".jsonl")):
        e = json.loads(line).get("entrypoint")
        if e:
            return e
    return None


# ---- a
f, note = notes_only("a")
note(argv=["claude"] + ARGS, cwd=cwd)
before = sessions()
t = Terminal(ARGS, cwd)
asked_trust = t.wait_text("Yes, I trust this folder", 25)
if asked_trust:  # the first choice is "No, exit": one step down, then Enter
    t.keys("\x1b[B")
    t.read(0.5)
    t.keys("\r")
t.read(4)
t.keys("Remember this secret word: meadow. Reply with the single word: noted")
t.read(1)
t.keys("\r")
sid, end = None, time.time() + 120
while time.time() < end and sid is None:
    t.read(1)
    new = [n for n in sessions() if n not in before]
    for n in new:
        if "assistant" in (transcript_types(n[:-len(".jsonl")]) or []):
            sid = n[:-len(".jsonl")]
t.read(2)
t.keys("/exit")
t.read(1)
t.keys("\r")
code = t.wait_exit(20)
if code is None:
    t.kill()
d = session_dir(cwd)
note(asked_whether_to_trust_the_folder=asked_trust, exit_code=code if code is not None else "killed after 20 s",
     sessions_folder=os.path.relpath(d, os.path.expanduser("~")) if d else None,
     session_files_before=before, session_files_after=sessions(), session_found=sid is not None,
     transcript_types=transcript_types(sid) if sid else None, entrypoint=entrypoint(sid) if sid else None,
     other_entries_in_the_folder=sorted(x for x in os.listdir(d) if not x.endswith(".jsonl")) if d else None)
f.close()
print("a: trust asked:", asked_trust, "exit:", code, "session:", sid, "files:", sessions())
if sid is None:
    raise SystemExit("the terminal session has no transcript: record again")

# ---- b
b = Session(NAME + "-b", repo=NAME, extra=["--tools", "Bash", "--resume", sid], session_id=None, hard_timeout=120)
ok = next_turn(b, "Without using any tool: which secret word did I ask you to remember in this conversation? Reply with that word, "
                  "or with the single word: none", 60)
init = b.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 1) or {}
r = last_result(b)
b.note(ok=ok, words_named=words_in(r.get("result"), ("meadow",)), init_session_id_is_the_terminal_session=init.get("session_id") == sid,
       result_session_id_is_the_terminal_session=r.get("session_id") == sid, result_subtype=r.get("subtype"))
print("b:", r.get("subtype"), r.get("result"), "same session:", r.get("session_id") == sid, "exit:", b.close(), "cost:", b.cost())

# ---- c
f, note = notes_only("c")
unknown = str(uuid.uuid4())
note(argv=["claude"] + ARGS + ["--resume", unknown], cwd=cwd)
t = Terminal(ARGS + ["--resume", unknown], cwd)
code = t.wait_exit(30)
if code is None:
    t.kill()
text = plain(t.screen).strip()
note(exit_code=code if code is not None else "killed after 30 s", printed=text[:300], printed_characters=len(text))
f.close()
print("c:", code, repr(text[:300]))
