"""D16: sandbox.failIfUnavailable.

argv[1]: available     {"enabled":true,"failIfUnavailable":true} on a machine where the sandbox works
         unavailable   the same plus "enabledPlatforms":["linux"]: on macOS the sandbox then counts as not available
         optional      {"enabled":true,"enabledPlatforms":["linux"]} without failIfUnavailable (control)
Only the --settings of these runs are changed; nothing on the machine is.
"""
import json, sys, time
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "available"
sandbox = {"enabled": True}
if variant != "optional":
    sandbox["failIfUnavailable"] = True
if variant != "available":
    sandbox["enabledPlatforms"] = ["linux"]
s = Session("r4-sandbox-required-" + variant, extra=["--tools", "Bash", "--settings", json.dumps({"sandbox": sandbox})], hard_timeout=90)
r = s.control("get_sandbox_dialog", wait=10)
print("get_sandbox_dialog ->", str(r and r["response"])[:600])
r = s.control("initialize", hooks=None, wait=10)
print("initialize answered:", bool(r))
time.sleep(1.0)
print("alive after 1 s:", s.alive())
if s.alive():
    s.user("Run exactly this Bash command: echo hi > inside.txt")
    print("result:", s.wait_results(1, 60))
    s.control("get_sandbox_dialog", wait=10)
print("cost:", s.cost(), "exit:", s.close())
