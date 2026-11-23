"""Scenario 4: the Agent tool (subagent) and how its messages are marked.

Variants (argv[1]): plain | forward (--forward-subagent-text) | foreground (prompt asks run_in_background:false)
| foreground-forward | stdin-closed (stdin closed right after the first result).
"""
import sys, time
from drv import Session
variant = sys.argv[1] if len(sys.argv) > 1 else "plain"
extra = ["--tools", "Bash,Read,Agent"]
if "forward" in variant:
    extra.append("--forward-subagent-text")
s = Session("subagent" + ("" if variant == "plain" else "-" + variant), extra=extra, hard_timeout=170, repo="subagent-" + variant)
prompt = "Use the Agent tool to have a subagent run `echo hi` with Bash and report its output. Then tell me what it reported."
if variant.startswith("foreground"):
    prompt = "Use the Agent tool with run_in_background set to false to have a subagent run `echo hi` with Bash and report its output. Then tell me what it reported."
s.user(prompt)
print("result 1:", s.wait_results(1, 150))
if variant != "stdin-closed":
    print("result 2:", s.wait_results(2, 30))
    time.sleep(2)
print("results:", s.count(lambda e: e.get("type") == "result"))
print("exit:", s.close(wait=60))
print("results at exit:", s.count(lambda e: e.get("type") == "result"))
