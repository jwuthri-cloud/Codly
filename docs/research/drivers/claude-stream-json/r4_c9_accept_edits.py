"""C9: --permission-mode acceptEdits. Do `mv` and `rm` in Bash ask? Does an Edit of .claude/settings.local.json ask?

The work dir gets a `.claude/settings.local.json` holding `{}` before the CLI starts. Every request is allowed.
"""
import os
from drv import Session, make_repo
cwd = make_repo("r4-accept-edits")
os.makedirs(os.path.join(cwd, ".claude"), exist_ok=True)
with open(os.path.join(cwd, ".claude", "settings.local.json"), "w") as f:
    f.write("{}\n")
seen = []

def handler(req):
    seen.append(req)
    return {"behavior": "allow", "updatedInput": req.get("input", {})}

s = Session("r4-accept-edits", mode="acceptEdits", extra=["--tools", "Bash,Read,Edit,Write"], hard_timeout=200, on_can_use_tool=handler)
n = 0
def turn(text):
    global n
    n += 1
    before = len(seen)
    s.user(text)
    ok = s.wait_results(n, 60)
    asked = [(r["tool_name"], r.get("input", {}).get("command") or r.get("input", {}).get("file_path")) for r in seen[before:]]
    s.note(turn=n, can_use_tool_requests=len(asked))
    print("turn %d ok=%s can_use_tool=%s" % (n, ok, asked))

turn("Use the Write tool to create the file note.txt with the content: hi")
turn("Run exactly this Bash command: mv hello.txt hello2.txt")
turn("Run exactly this Bash command: rm hello2.txt")
turn("Run exactly this Bash command: rm -rf sub && mkdir sub && mv note.txt sub/a && mv sub/a sub/b")
turn("Use the Read tool on .claude/settings.local.json, then use the Edit tool to replace `{}` with `{ }` in that file.")
s.note(files=sorted(os.listdir(cwd)), settings_local=open(os.path.join(cwd, ".claude", "settings.local.json")).read())
print("cost:", s.cost(), "exit:", s.close())
