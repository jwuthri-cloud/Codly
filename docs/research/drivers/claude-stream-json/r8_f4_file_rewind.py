"""F4: Claude Code's own file rewind (`rewind_files`). Part 1 got "File rewinding is not enabled." for a dry run.

The bundle (bgrep.py "File rewinding is not enabled", then the function it calls) gates it: in a run that is not
interactive only the variable CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING turns it on (and
CLAUDE_CODE_DISABLE_FILE_CHECKPOINTING off); the setting `fileCheckpointingEnabled` counts only in the terminal UI.
This script records the three ways.

argv[1]: env      the variable CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=1 in the CLI's environment
         setting  `--settings '{"fileCheckpointingEnabled": true}'`, no variable
         off      neither
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.

One process, `--tools Read,Edit`, mode acceptEdits. Turn 1 (user message uuid U1): an Edit of hello.txt from `hello`
to `hello one`. Turn 2 (uuid U2): `hello one` -> `hello two`. Then, each followed by a note with the text of hello.txt:
rewind_files U2 dry run, U2, U1 dry run, U1, a uuid that no message has (dry run). For `env`, after the exit:
`claude -p --resume <session> --rewind-files <U2>` (the flag of `claude --help`, "Restore files ... and exit") with
the same variable, its exit code and first line of output, and the text of hello.txt.
"""
import json, os, sys, uuid
from drv import Session
from r5_lib import ask, next_turn
from r7_lib import claude_cli
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "env"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
ENV = {"env": {"CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING": "1"}, "setting": {}, "off": {}}[VARIANT]
extra = ["--tools", "Read,Edit"] + (["--settings", json.dumps({"fileCheckpointingEnabled": True})] if VARIANT == "setting" else [])
s = Session("r8-file-rewind-%s-%s" % (VARIANT, LABEL), extra=extra, hard_timeout=150, env=ENV or None)
hello = os.path.join(s.cwd, "hello.txt")
with open(hello, "w") as f:
    f.write("hello\n")


def text():
    return open(hello).read().strip()


u1, u2 = str(uuid.uuid4()), str(uuid.uuid4())
for u, old, new in ((u1, "hello", "hello one"), (u2, "hello one", "hello two")):
    ok = next_turn(s, "Use the Edit tool to change the text of hello.txt from `%s` to `%s` (read the file first). Then reply with the single word: done" % (old, new), 60, uuid=u)
    s.note(turn=new, ok=ok, hello_txt=text())
for label, u, dry in (("U2 dry run", u2, True), ("U2", u2, False), ("U1 dry run", u1, True), ("U1", u1, False), ("unknown uuid dry run", str(uuid.uuid4()), True)):
    r = ask(s, "rewind_files", user_message_id=u, dry_run=dry)
    s.note(after="rewind_files " + label, answer_subtype=r.get("subtype"), hello_txt=text())
sid = s.sid


def after_exit():
    if VARIANT != "env":
        return dict(hello_txt_at_the_end=text())
    r = claude_cli(["-p", "--resume", sid, "--rewind-files", u2], s.cwd, ENV)
    out = (r.stdout.strip() or r.stderr.strip()).splitlines()
    return dict(rewind_files_flag=dict(command="claude -p --resume <session> --rewind-files <U2>", exit_code=r.returncode,
                                       first_line_of_its_output=out[0] if out else "", hello_txt=text()))


print("cost:", s.cost(), "exit:", s.close(after_exit=after_exit), "hello.txt:", text())
