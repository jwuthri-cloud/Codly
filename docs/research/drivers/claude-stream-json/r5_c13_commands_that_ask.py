"""C13 / D5: which Bash commands ask (`can_use_tool`) in acceptEdits and in the app's `manual` mode, with the sandbox
off and with the sandbox on + autoAllowBashIfSandboxed:false. The same commands in every run, one per turn:
a write, `git add` + `git commit`, `git push` to a bare repo inside the work folder, `curl` to example.com,
`chmod +x`, `rm` of a tracked file, a Write-tool write and a shell write to `.claude/settings.local.json`.

argv[1]: off            acceptEdits, no sandbox
         noauto         acceptEdits, sandbox {"enabled":true,"autoAllowBashIfSandboxed":false}
         manual-off     manual, no sandbox
         manual-noauto  manual, the same sandbox settings
Every request is allowed (a request to run outside the sandbox is denied).
"""
import json, os, sys
from drv import Session, make_repo
from r5_lib import allow_all, asked, git, results
variant = sys.argv[1] if len(sys.argv) > 1 else "off"
name = "r5-commands-" + variant
cwd = make_repo(name)
if not os.path.isdir(os.path.join(cwd, "remote.git")):
    git(cwd, "init", "-q", "--bare", "remote.git")
    git(cwd, "remote", "add", "origin", "./remote.git")
    git(cwd, "config", "user.name", "demo")
    git(cwd, "config", "user.email", "demo@example.invalid")
    git(cwd, "config", "commit.gpgsign", "false")
    os.makedirs(os.path.join(cwd, ".git", "info"), exist_ok=True)
    with open(os.path.join(cwd, ".git", "info", "exclude"), "a") as f:
        f.write("remote.git\n.claude\n")
    git(cwd, "push", "-q", "origin", "HEAD")
os.makedirs(os.path.join(cwd, ".claude"), exist_ok=True)
local = os.path.join(cwd, ".claude", "settings.local.json")
with open(local, "w") as f:
    f.write("{}\n")
extra = ["--tools", "Bash,Read,Edit,Write"]
if variant.endswith("noauto"):
    extra += ["--settings", json.dumps({"sandbox": {"enabled": True, "autoAllowBashIfSandboxed": False}})]
seen = []
s = Session(name, mode="manual" if variant.startswith("manual") else "acceptEdits", extra=extra, hard_timeout=280,
            on_can_use_tool=allow_all(seen))
if variant.endswith("noauto"):
    s.control("get_sandbox_dialog")

def turn(text):
    before, n = len(seen), results(s)
    s.user(text)
    ok = s.wait_results(n + 1, 70)
    s.note(turn=n + 1, can_use_tool_requests=asked(seen[before:]))
    print("turn %d ok=%s can_use_tool=%s" % (n + 1, ok, asked(seen[before:])))

for cmd in ("echo hi > note.txt",
            "git add note.txt && git commit -m note",
            "git push origin HEAD",
            "curl -sI https://example.com",
            "chmod +x hello.txt",
            "rm hello.txt"):
    turn("Run exactly this Bash command once (do not retry it, do not change it): " + cmd)
# The Write tool first, and with new content: after a shell write that the sandbox blocked, and for content the file
# already had, the model did not call the tool at all.
turn("Use the Write tool to overwrite the file .claude/settings.local.json with exactly this content (it differs from "
     "what is there, and I do want it changed): { }")
turn("Run exactly this Bash command once (do not retry it, do not change it): echo '{  }' > .claude/settings.local.json")
s.note(files=sorted(f for f in os.listdir(cwd) if not f.startswith(".") and f != "remote.git"),
       hello_is_executable=os.path.exists(os.path.join(cwd, "hello.txt")) and os.access(os.path.join(cwd, "hello.txt"), os.X_OK),
       commits_here=git(cwd, "rev-list", "--count", "HEAD").stdout.strip(),
       commits_in_the_bare_repo=git(os.path.join(cwd, "remote.git"), "rev-list", "--count", "--all").stdout.strip(),
       settings_local=open(local).read())
print("cost:", s.cost(), "exit:", s.close())
