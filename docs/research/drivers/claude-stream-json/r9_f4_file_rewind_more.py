"""F4: Claude Code's own file rewind (`rewind_files`) beyond the Edit tool, with
CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1 (part 6, section 4: only this variable turns it on in a run that is not
interactive).

One process, `--tools Bash,Read,Edit,Write,Agent`, mode acceptEdits, every request allowed. The work folder has
hello.txt (`hello`), notes.txt (`notes`) and README.md (first line `# demo`).
Turn 1 (host uuid U1): Bash writes notes.txt (`bash wrote this`), the Write tool makes new.txt (`new file`), and a
helper (Agent) edits README.md's first line to `# demo edited by helper`. A note: the three files after the turn,
and the tools of the main thread and of the helper.
Then: `rewind_files U1` dry run, then the real one, each followed by a note with the three files; then the line
types the transcript got from the rewind (r9_lib.transcript_types: the CLI's own words, nothing of the
conversation). Turn 2 (no tools): "did anything tell you that files were put back?"; a note: its answer, the
transcript's new line types, the stream's lines of the turn.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import os, sys, time, uuid
from drv import Session
from r5_lib import ask, next_turn, open_tasks
from r6_lib import kinds
from r8_lib import last_result
from r9_lib import transcript_types
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
s = Session("r9-file-rewind-more-" + LABEL, extra=["--tools", "Bash,Read,Edit,Write,Agent"], hard_timeout=240,
            env={"CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING": "1"})
FILES = ("notes.txt", "new.txt", "README.md")
for f, text in (("hello.txt", "hello\n"), ("notes.txt", "notes\n"), ("README.md", "# demo\n\nA tiny demo repo.\n")):
    with open(os.path.join(s.cwd, f), "w") as fh:
        fh.write(text)
new = os.path.join(s.cwd, "new.txt")
if os.path.exists(new):
    os.remove(new)


def state():
    """The first line of each file, or None when it does not exist."""
    out = {}
    for f in FILES:
        p = os.path.join(s.cwd, f)
        out[f] = open(p).read().splitlines()[0] if os.path.exists(p) else None
    return out


def lines():
    return len(transcript_types(s.sid) or [])


u1 = str(uuid.uuid4())
m = s.mark()
ok = next_turn(s, "Do these three steps, in this order, then reply with the single word: done. "
                  "1. Run the Bash command `printf 'bash wrote this\\n' > notes.txt`. "
                  "2. Use the Write tool to create the file new.txt with the text `new file`. "
                  "3. Use the Agent tool once, in the foreground (not in the background), and wait for its answer: ask a helper to read "
                  "README.md and use the Edit tool to change its first line from `# demo` to `# demo edited by helper`.", 180, uuid=u1)
# The rewind must come after everything turn 1 started: wait until no background task is listed and no turn runs.
end = time.time() + 90
while time.time() < end and (open_tasks(s) or s.count(lambda e: e.get("type") == "result") < s.count(lambda e: e.get("type") == "system" and e.get("subtype") == "init")):
    time.sleep(0.5)
with s.cv:
    used = [[b.get("name"), bool(e.get("parent_tool_use_id"))] for e in s.events[m:] if e.get("type") == "assistant"
            for b in e["message"]["content"] if b.get("type") == "tool_use"]
    agent_background = [bool(b["input"].get("run_in_background")) for e in s.events[m:] if e.get("type") == "assistant"
                        for b in e["message"]["content"] if b.get("type") == "tool_use" and b.get("name") == "Agent"]
s.note(turn=1, ok=ok, files=state(), tools_used_and_by_a_helper=used, agent_call_in_the_background=agent_background,
       results_so_far=s.count(lambda e: e.get("type") == "result"))
before = lines()
for label, dry in (("U1 dry run", True), ("U1", False)):
    r = ask(s, "rewind_files", user_message_id=u1, dry_run=dry)
    s.note(after="rewind_files " + label, answer_subtype=r.get("subtype"), files=state())
time.sleep(1.0)
s.note(transcript_types_added_by_the_rewind=(transcript_types(s.sid) or [])[before:])
before = lines()
m = s.mark()
ok = next_turn(s, "Without using any tool: since your last turn, did any message or note tell you that files in this folder were put back "
                  "to an earlier state? Reply with yes or no, and if yes, which files.", 60, uuid=str(uuid.uuid4()))
time.sleep(0.5)
s.note(turn=2, ok=ok, answer=last_result(s, m).get("result"), transcript_types_added_by_the_turn=(transcript_types(s.sid) or [])[before:],
       lines=kinds(s, m), files=state())
print("files:", state(), "cost:", s.cost(), "exit:", s.close())
