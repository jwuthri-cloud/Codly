"""Scenario 9: --settings '{"sandbox":{"enabled":true}}' in -p mode.

argv[1]: allow (answer every can_use_tool with allow) | deny-unsandboxed (deny requests whose input has
dangerouslyDisableSandbox) | strict (settings also set allowUnsandboxedCommands:false) | off (no sandbox, control run)
The outside probe is /tmp/sandbox-probe-outside-<variant>.txt (removed afterwards).
"""
import json, os, sys, time
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "allow"
PROBE = "/tmp/sandbox-probe-outside-%s.txt" % variant
settings = {"sandbox": {"enabled": True}}
if variant == "strict":
    settings["sandbox"]["allowUnsandboxedCommands"] = False
extra = ["--tools", "Bash"]
if variant != "off":
    extra += ["--settings", json.dumps(settings)]

def handler(req):
    inp = req.get("input", {})
    if variant == "deny-unsandboxed" and inp.get("dangerouslyDisableSandbox"):
        return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}
    return {"behavior": "allow", "updatedInput": inp}

if os.path.exists(PROBE):
    os.remove(PROBE)
s = Session("sandbox-" + variant, extra=extra, hard_timeout=170, on_can_use_tool=handler)
s.control("get_sandbox_dialog")
s.user("Run exactly this Bash command and report what happened: echo hi > inside.txt")
print("result 1:", s.wait_results(1, 60))
print("inside.txt exists:", os.path.exists(os.path.join(s.cwd, "inside.txt")))
s.user("Run exactly this Bash command and report what happened: touch " + PROBE)
print("result 2:", s.wait_results(2, 90))
print("probe exists:", os.path.exists(PROBE))
s._log("meta", {"after_turn_2_outside_probe_exists": os.path.exists(PROBE)})
if variant != "off":
    s.user("Run the same touch command once more, this time with the Bash tool parameter dangerouslyDisableSandbox set to true. Report what happened.")
    print("result 3:", s.wait_results(3, 90))
    print("probe exists:", os.path.exists(PROBE))
s._log("meta", {"inside_exists": os.path.exists(os.path.join(s.cwd, "inside.txt")), "outside_probe_exists": os.path.exists(PROBE)})
print("exit:", s.close())
if os.path.exists(PROBE):
    os.remove(PROBE)
print("probe removed:", not os.path.exists(PROBE))
