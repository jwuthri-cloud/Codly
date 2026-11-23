"""Scenario 12 (extra recording): hook events, prompt suggestions, replayed user messages, tool_progress,
control_cancel_request (interrupt while a permission request waits) and deny with interrupt:true.

A PreToolUse and a Stop hook are set in the throw-away repo's .claude/settings.local.json.
"""
import json, os, time
from drv import Session, make_repo
repo = make_repo("misc")
os.makedirs(os.path.join(repo, ".claude"), exist_ok=True)
with open(os.path.join(repo, ".claude", "settings.local.json"), "w") as f:
    json.dump({"hooks": {
        "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "echo pre-tool-hook-ran"}]}],
        "Stop": [{"hooks": [{"type": "command", "command": "echo stop-hook-ran"}]}],
    }}, f)
state = {"mode": "allow"}
pending = []

def handler(req):
    if state["mode"] == "allow":
        return {"behavior": "allow", "updatedInput": req.get("input", {})}
    if state["mode"] == "hold":
        pending.append(req)
        return None                      # leave the request unanswered
    return {"behavior": "deny", "message": "The user stopped this turn.", "interrupt": True}

s = Session("misc-events", mode="default", extra=["--tools", "Bash", "--include-hook-events", "--prompt-suggestions", "true", "--replay-user-messages"],
            hard_timeout=200, on_can_use_tool=handler, repo="misc")
s.user("Run exactly this Bash command, then reply with the single word ok: sleep 12; echo slept")
print("result 1:", s.wait_results(1, 60)); time.sleep(3)
state["mode"] = "hold"
s.user("Run exactly this Bash command: echo x > f.txt")
s.wait_for(lambda e: e.get("type") == "control_request", 40)
time.sleep(1.0)
s.control("interrupt")
print("result 2:", s.wait_results(2, 30)); time.sleep(3)
state["mode"] = "deny-interrupt"
s.user("Run exactly this Bash command: echo y > g.txt")
print("result 3:", s.wait_results(3, 40)); time.sleep(3)
print("exit:", s.close())
