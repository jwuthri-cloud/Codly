"""D14: sandbox on, one `curl -sI https://example.com`. The request the host gets, and what follows.

argv[1]: allow | deny   (the answer to the sandbox's network request; every other request is allowed, except a
request to run outside the sandbox, which is always denied so that only the sandboxed path is recorded)
"""
import json, sys
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "allow"
seen = []

def handler(req):
    seen.append(req)
    inp = req.get("input", {})
    if inp.get("dangerouslyDisableSandbox"):
        return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
    if req["tool_name"] != "Bash" and variant == "deny":
        return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
    return {"behavior": "allow", "updatedInput": inp}

s = Session("r4-sandbox-network-" + variant, extra=["--tools", "Bash", "--settings", json.dumps({"sandbox": {"enabled": True}})],
            hard_timeout=150, on_can_use_tool=handler)
s.control("get_sandbox_dialog")
s.user("Run exactly this Bash command once (do not retry it, do not change it) and report the first line of its output: curl -sI https://example.com")
print("result:", s.wait_results(1, 100))
print("can_use_tool:", [(r["tool_name"], r.get("input")) for r in seen])
s.control("get_sandbox_dialog")
s.control("list_permission_rules")
print("cost:", s.cost(), "exit:", s.close())
