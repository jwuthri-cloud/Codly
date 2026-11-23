"""D6: sandbox on; `curl` to example.com is allowed once; does a second command to the same host ask again?

Turn 1 and turn 2: `curl -sI https://example.com`. Turn 3: `curl -sI https://example.org` (another host).
After each turn: get_sandbox_dialog and list_permission_rules.

argv[1]: once          the network request is answered with a plain allow (what "allow once" sends)
         rule-local    allow + updatedPermissions = the request's permission_suggestions, as the CLI sent them
         rule-session  the same with "destination":"session"
"""
import copy, json, os, sys
from drv import Session
from r5_lib import asked, results
variant = sys.argv[1] if len(sys.argv) > 1 else "once"
seen = []

def handler(req):
    seen.append(req)
    inp = req.get("input", {})
    if inp.get("dangerouslyDisableSandbox"):
        return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
    r = {"behavior": "allow", "updatedInput": inp}
    if req["tool_name"] == "SandboxNetworkAccess" and variant != "once" and inp.get("host") == "example.com":
        sug = copy.deepcopy(req.get("permission_suggestions") or [])
        if variant == "rule-session":
            for x in sug:
                x["destination"] = "session"
        r["updatedPermissions"] = sug
    return r

s = Session("r5-sandbox-network-" + variant, extra=["--tools", "Bash", "--settings", json.dumps({"sandbox": {"enabled": True}})],
            hard_timeout=200, on_can_use_tool=handler)
local = os.path.join(s.cwd, ".claude", "settings.local.json")
s.control("get_sandbox_dialog")

def turn(cmd):
    before, n = len(seen), results(s)
    s.user("Run exactly this Bash command once (do not retry it, do not change it) and report the first line of its output: " + cmd)
    ok = s.wait_results(n + 1, 70)
    s.note(turn=n + 1, can_use_tool_requests=asked(seen[before:]),
           settings_local_json=json.load(open(local)) if os.path.exists(local) else None)
    print("turn %d ok=%s can_use_tool=%s" % (n + 1, ok, asked(seen[before:])))
    s.control("get_sandbox_dialog")
    s.control("list_permission_rules")

turn("curl -sI https://example.com")
turn("curl -sI https://example.com")
turn("curl -sI https://example.org")
print("cost:", s.cost(), "exit:", s.close())
