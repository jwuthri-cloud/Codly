"""D15: do sandboxed Bash commands ask through can_use_tool?

argv[1]: noauto         acceptEdits + {"enabled":true,"autoAllowBashIfSandboxed":false}
         manual         manual      + {"enabled":true}                                   (the app's 1:1 agent)
         manual-noauto  manual      + {"enabled":true,"autoAllowBashIfSandboxed":false}
Every request is allowed.
"""
import json, os, sys
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "noauto"
sandbox = {"enabled": True}
if variant.endswith("noauto"):
    sandbox["autoAllowBashIfSandboxed"] = False
mode = "manual" if variant.startswith("manual") else "acceptEdits"
seen = []

def handler(req):
    seen.append(req)
    return {"behavior": "allow", "updatedInput": req.get("input", {})}

s = Session("r4-sandbox-" + variant, mode=mode, extra=["--tools", "Bash", "--settings", json.dumps({"sandbox": sandbox})],
            hard_timeout=150, on_can_use_tool=handler)
s.control("get_sandbox_dialog")
n = 0
def turn(text):
    global n
    n += 1
    before = len(seen)
    s.user(text)
    ok = s.wait_results(n, 60)
    asked = [(r["tool_name"], r.get("input", {}).get("command"), r.get("decision_reason_type")) for r in seen[before:]]
    s.note(turn=n, can_use_tool_requests=len(asked))
    print("turn %d ok=%s can_use_tool=%s" % (n, ok, asked))
turn("Run exactly this Bash command: echo hi > inside.txt")
turn("Run exactly this Bash command: ls")
turn("Run exactly this Bash command: mv inside.txt inside2.txt")
s.note(files=sorted(f for f in os.listdir(s.cwd) if not f.startswith(".")))
print("cost:", s.cost(), "exit:", s.close())
