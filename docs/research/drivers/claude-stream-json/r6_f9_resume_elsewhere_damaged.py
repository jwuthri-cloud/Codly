"""F9: `--resume` of a session from ANOTHER FOLDER, and of a session whose transcript file is DAMAGED.

First a source session in work/r6-resume-source: two tiny turns (the answers are `apple` and `pear`).
Then, three runs each (the host writes `initialize` right after the spawn, as in r5_f9):
  r6-resume-other-folder-1 … -3   `--resume <the source id>` started in work/r6-resume-elsewhere
  r6-resume-damaged-tail-1 … -3   `--resume <id of a copy>` in the source folder; the copy's LAST line is cut in half
  r6-resume-damaged-mid-1 … -3    the same; the copy ends in the middle of the line that holds the SECOND message
                                  (so the first turn is whole and the second is lost)
A run whose process answers `initialize` and stays gets one message that asks for the words said before, so that the
recording shows which conversation it holds.

The copies: the CLI keeps a session in `~/.claude/projects/<folder name>/<session id>.jsonl`. The script copies the
source session's file to a new id in the same folder (the old id inside the lines is replaced by the new one), cuts
the copy, and deletes the copies at the end. It never changes another file there.
"""
import glob, json, os, time, uuid
from drv import Session
from r5_lib import next_turn

src = Session("r6-resume-source", extra=["--tools", "Bash"], hard_timeout=90)
for word in ("apple", "pear"):
    print("source turn:", next_turn(src, "Reply with the single word: " + word))
print("source cost:", src.cost(), "exit:", src.close())
found = glob.glob(os.path.join(os.path.expanduser("~"), ".claude", "projects", "*", src.sid + ".jsonl"))
if len(found) != 1:
    raise SystemExit("the source session's transcript file was not found: %r" % found)
transcript = found[0]
whole = open(transcript, "rb").read()
print("transcript: %d bytes, %d lines" % (len(whole), whole.count(b"\n")))


def damaged_copy(kind):
    """A copy of the source transcript under a new session id, cut. Returns (id, path, what was done)."""
    sid = str(uuid.uuid4())
    data = whole.replace(src.sid.encode(), sid.encode())
    lines = data.splitlines(keepends=True)
    parsed = [json.loads(l) for l in lines]
    if kind == "tail":
        victim = len(lines) - 1
    else:
        victim = [i for i, d in enumerate(parsed) if d.get("type") == "user" and "pear" in json.dumps(d.get("message"))][0]
    cut = sum(len(l) for l in lines[:victim]) + len(lines[victim]) // 2
    path = os.path.join(os.path.dirname(transcript), sid + ".jsonl")
    with open(path, "wb") as f:
        f.write(data[:cut])
    kept = [d.get("type") for d in parsed[:victim]]
    return sid, path, dict(lines_of_the_source=len(lines), whole_lines_kept=victim, kind_of_the_cut_line=parsed[victim].get("type"),
                           bytes_kept_of_the_cut_line=len(lines[victim]) // 2, bytes_of_the_cut_line=len(lines[victim]),
                           whole_user_lines_kept=kept.count("user"), whole_assistant_lines_kept=kept.count("assistant"))


def file_state(path):
    """What the copy looks like now: its size, and how many of its lines are not JSON."""
    lines = open(path, "rb").read().splitlines()
    bad = 0
    for l in lines:
        try:
            json.loads(l)
        except ValueError:
            bad += 1
    return dict(the_copy_after_the_run=dict(lines=len(lines), lines_that_are_not_json=bad))


def session_files(sid):
    """Where the CLI keeps this session after the run: project folder name -> lines of its file."""
    found = glob.glob(os.path.join(os.path.expanduser("~"), ".claude", "projects", "*", sid + ".jsonl"))
    return dict(session_files_with_this_id={os.path.basename(os.path.dirname(f)): len(open(f, "rb").read().splitlines()) for f in found})


def run(name, sid, repo, about, copy=None):
    s = Session(name, extra=["--resume", sid, "--tools", "Bash"], session_id=None, hard_timeout=90, repo=repo)
    s.control("initialize", wait=0, hooks=None)
    answered = s.wait_for(lambda e: e.get("type") == "control_response", 20)
    time.sleep(1.0)
    turn = None
    if answered and s.alive():
        turn = next_turn(s, "Which single words have you replied with so far in this conversation? Reply with those words only, "
                            "in order, or with the word nothing.", 40)
    with s.cv:
        kinds = ["initialize answer (%s)" % e["response"].get("subtype") if e.get("type") == "control_response"
                 else "%s/%s" % (e.get("type"), e.get("subtype")) for e in s.events if e.get("type") in ("control_response", "result") or e.get("subtype") == "init"]
        res = [e for e in s.events if e.get("type") == "result"]
    s.note(about_the_session=about, initialize_answered=bool(answered), stdout_lines_in_order=kinds, exited_by_itself=not s.alive(), a_turn_was_sent=turn is not None,
           turn_result=res[-1].get("result") if res else None, result_session_id_is_the_resumed_one=(res[-1].get("session_id") == sid) if res else None)
    code = s.close(after_exit=lambda: file_state(copy) if copy else session_files(sid))
    print("%s: initialize answered: %s; lines: %s; result: %r; exit code: %s; %s" % (name, bool(answered), kinds, res[-1].get("result") if res else None, code,
                                                                                   json.dumps(file_state(copy) if copy else session_files(sid))))


copies = []
try:
    for i in (1, 2, 3):
        run("r6-resume-other-folder-%d" % i, src.sid, "r6-resume-elsewhere", dict(kind="a whole session of another folder"))
    for kind in ("tail", "mid"):
        for i in (1, 2, 3):
            sid, path, about = damaged_copy(kind)
            copies.append(path)
            run("r6-resume-damaged-%s-%d" % (kind, i), sid, "r6-resume-source", dict(kind="a damaged copy (%s)" % kind, **about), copy=path)
finally:
    for path in copies:
        if os.path.exists(path):
            os.remove(path)
