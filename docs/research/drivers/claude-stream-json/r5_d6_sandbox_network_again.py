"""D6: sandbox on; `curl` to example.com is allowed once; does a second command to the same host ask again?

Turn 1 and turn 2: `curl -sI https://example.com`. Turn 3: `curl -sI https://example.org` (another host).
After each turn: get_sandbox_dialog and list_permission_rules.

argv[1]: once          the network request is answered with a plain allow (what "allow once" sends)
         once-later    the same, but between the two `curl` to example.com the session runs another turn
                       (`echo between`) and then waits 300 s (so the run has four turns and takes about 6 minutes)
         once-restart  two processes (fixtures `…-once-restart-a` and `-b`): a plain allow in the first one, which is then
                       closed; the second one is `--resume` of that session in the same folder and runs the `curl` again
         rule-local    allow + updatedPermissions = the request's permission_suggestions, as the CLI sent them
         rule-session  the same with "destination":"session"
"""
import copy, json, os, sys, time
from drv import Session
from r5_lib import asked, results
variant = sys.argv[1] if len(sys.argv) > 1 else "once"
WAIT = 300 if variant == "once-later" else 0
seen = []

def handler(req):
    seen.append(req)
    inp = req.get("input", {})
    if inp.get("dangerouslyDisableSandbox"):
        return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
    r = {"behavior": "allow", "updatedInput": inp}
    if req["tool_name"] == "SandboxNetworkAccess" and variant.startswith("rule-") and inp.get("host") == "example.com":
        sug = copy.deepcopy(req.get("permission_suggestions") or [])
        if variant == "rule-session":
            for x in sug:
                x["destination"] = "session"
        r["updatedPermissions"] = sug
    return r

def start(name, resume=None, repo=None):
    extra = ["--tools", "Bash", "--settings", json.dumps({"sandbox": {"enabled": True}})]
    s = Session(name, extra=extra + (["--resume", resume] if resume else []), session_id=None if resume else "new",
                hard_timeout=200 + WAIT, on_can_use_tool=handler, repo=repo)
    s.control("get_sandbox_dialog")
    return s

def turn(s, cmd):
    before, n = len(seen), results(s)
    local = os.path.join(s.cwd, ".claude", "settings.local.json")
    s.user("Run exactly this Bash command once (do not retry it, do not change it) and report the first line of its output: " + cmd)
    ok = s.wait_results(n + 1, 70)
    s.note(turn=n + 1, can_use_tool_requests=asked(seen[before:]),
           settings_local_json=json.load(open(local)) if os.path.exists(local) else None)
    print("turn %d ok=%s can_use_tool=%s" % (n + 1, ok, asked(seen[before:])))
    s.control("get_sandbox_dialog")
    s.control("list_permission_rules")

if variant == "once-restart":
    first = start("r5-sandbox-network-once-restart-a")
    turn(first, "curl -sI https://example.com")
    print("cost:", first.cost(), "exit:", first.close())
    s = start("r5-sandbox-network-once-restart-b", resume=first.sid, repo="r5-sandbox-network-once-restart-a")
    turn(s, "curl -sI https://example.com")
else:
    s = start("r5-sandbox-network-" + variant)
    turn(s, "curl -sI https://example.com")
    if variant == "once-later":
        turn(s, "echo between")
        time.sleep(WAIT)
    turn(s, "curl -sI https://example.com")
    turn(s, "curl -sI https://example.org")
print("cost:", s.cost(), "exit:", s.close())
