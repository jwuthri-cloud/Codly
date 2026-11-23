"""Scenario 10 (part 2): allow + updatedPermissions taken from permission_suggestions; is the same command asked again?"""
import copy, json, os
from drv import Session
seen = []
policy = {"n": 0}

def handler(req):
    seen.append(req)
    sug = copy.deepcopy(req.get("permission_suggestions") or [])
    cmd = req.get("input", {}).get("command", "")
    r = {"behavior": "allow", "updatedInput": req.get("input", {})}
    if cmd.startswith("git add"):
        for x in sug:
            x["destination"] = "session"      # every suggestion, scoped to the session
        r["updatedPermissions"] = sug
    elif cmd.startswith("python3") or cmd.startswith("touch"):
        r["updatedPermissions"] = sug          # every suggestion, exactly as the CLI sent it
    return r

s = Session("permission-updated-permissions", mode="default", extra=["--tools", "Bash,Read,Edit"], hard_timeout=230, on_can_use_tool=handler, repo="permissions2")
n = 0
def turn(text):
    global n
    n += 1
    before = len(seen)
    s.user(text)
    ok = s.wait_results(n, 60)
    print("turn %d ok=%s can_use_tool=%s" % (n, ok, [(r["tool_name"], r.get("input", {}).get("command") or r.get("input", {}).get("file_path"), [x["type"] + ":" + x.get("destination", "") for x in r.get("permission_suggestions", [])]) for r in seen[before:]]))
turn("Run exactly this Bash command: git add -A")
turn("Run exactly this Bash command again: git add -A")
turn("Run exactly this Bash command: python3 -c 'print(42)'")
turn("Run exactly this Bash command again: python3 -c 'print(42)'")
p = os.path.join(s.cwd, ".claude", "settings.local.json")
s._log("meta", {"settings_local_json": json.load(open(p)) if os.path.exists(p) else None})
turn("Run exactly this Bash command: touch newfile.txt")
turn("Use the Edit tool to change the text of hello.txt from `hello` to `hello world` (read the file first).")
s.control("list_permission_rules")
s._log("meta", {"settings_local_json": json.load(open(p)) if os.path.exists(p) else None})
print("exit:", s.close())
