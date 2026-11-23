"""Scenario 10: can_use_tool request shapes.

argv[1]: default (Bash / Edit / outside-cwd / deny / updatedPermissions) | plan (ExitPlanMode) | ask (AskUserQuestion)
"""
import copy, json, os, sys, time
from drv import Session
which = sys.argv[1]
MODE = sys.argv[2] if len(sys.argv) > 2 else "default"
PROBE = "/tmp/perm-probe-outside.txt"
seen = []

def allow(req, **more):
    r = {"behavior": "allow", "updatedInput": req.get("input", {})}
    r.update(more)
    return r

def deny(msg):
    return {"behavior": "deny", "message": msg, "interrupt": False}

if which == "default":
    def handler(req):
        seen.append(req)
        cmd = req.get("input", {}).get("command", "")
        if req["tool_name"] == "Bash" and cmd.startswith("echo probe"):
            # allow + persist the CLI's own suggestion, scoped to the session
            sug = copy.deepcopy(req.get("permission_suggestions") or [])
            for x in sug:
                x["destination"] = "session"
            return allow(req, updatedPermissions=sug) if sug else allow(req)
        if req["tool_name"] == "Bash" and cmd.startswith("rm "):
            return deny("The user denied this action.")
        return allow(req)
    s = Session("permission-request-shapes", mode=MODE, extra=["--tools", "Bash,Read,Edit"], hard_timeout=230, on_can_use_tool=handler, repo="permissions")
    n = 0
    def turn(text):
        global n
        n += 1
        before = len(seen)
        s.user(text)
        ok = s.wait_results(n, 60)
        print("turn %d ok=%s can_use_tool=%s" % (n, ok, [(r["tool_name"], r.get("input", {}).get("command") or r.get("input", {}).get("file_path")) for r in seen[before:]]))
    turn("Run exactly this Bash command: git status")
    turn("Run exactly this Bash command: echo probe > note.txt")
    turn("Run exactly this Bash command again: echo probe > note.txt")
    s.control("list_permission_rules")
    turn("Use the Edit tool to change the text of hello.txt from `hello` to `hello world` (read the file first).")
    turn("Run exactly this Bash command: touch " + PROBE)
    turn("Use the Read tool to read the file /etc/hosts and tell me how many lines it has.")
    turn("Run exactly this Bash command: rm note.txt")
    s._log("meta", {"note_exists_after_denied_rm": os.path.exists(os.path.join(s.cwd, "note.txt")), "outside_probe_exists": os.path.exists(PROBE),
                    "settings_local_exists": os.path.exists(os.path.join(s.cwd, ".claude", "settings.local.json"))})
    print("exit:", s.close())
    if os.path.exists(PROBE):
        os.remove(PROBE)
elif which == "plan":
    state = {"exit_plan": 0}
    def handler(req):
        seen.append(req)
        if req["tool_name"] == "ExitPlanMode":
            state["exit_plan"] += 1
            if state["exit_plan"] == 1:
                return deny("Not yet: add a final step that reads the file back to check it. Then present the plan again.")
            return allow(req)
        return allow(req)
    s = Session("permission-plan-mode", mode="plan", extra=["--tools", "Bash,Read,Edit,Write,ExitPlanMode"], hard_timeout=230, on_can_use_tool=handler, repo="plan")
    s.user("Plan a tiny change: add a second line `world` to hello.txt. Keep the plan to 2-3 short steps and present it with ExitPlanMode. When the plan is approved, do it.")
    print("result:", s.wait_results(1, 180))
    print("can_use_tool:", [(r["tool_name"], list(r.get("input", {}).keys())) for r in seen])
    s.control("get_plan")
    print("hello.txt:", repr(open(os.path.join(s.cwd, "hello.txt")).read()))
    print("exit:", s.close())
else:
    def handler(req):
        seen.append(req)
        if req["tool_name"] == "AskUserQuestion":
            inp = copy.deepcopy(req["input"])
            q = inp["questions"][0]
            pick = [o["label"] for o in q["options"] if "blue" in o["label"].lower()] or [q["options"][-1]["label"]]
            inp["answers"] = {q["question"]: pick[0]}
            return {"behavior": "allow", "updatedInput": inp}
        return allow(req)
    s = Session("permission-askuserquestion", mode="default", extra=["--tools", "Read,AskUserQuestion"], hard_timeout=120, on_can_use_tool=handler, repo="ask")
    s.user("Use the AskUserQuestion tool to ask me one question: which colour do I prefer, with the two options Red and Blue. Then reply with only the colour I chose.")
    print("result:", s.wait_results(1, 90))
    print("can_use_tool:", [(r["tool_name"], list(r.get("input", {}).keys())) for r in seen])
    print("exit:", s.close())
