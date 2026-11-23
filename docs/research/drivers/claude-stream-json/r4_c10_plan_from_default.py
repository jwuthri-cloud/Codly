"""C10 + C11: a session started as the app starts a 1:1 agent (mode `manual`, the app's --tools list), put in plan
mode with set_permission_mode. First ExitPlanMode: deny with a message. Second: allow with a CHANGED plan in updatedInput.
"""
import json, os
from drv import Session
# tools_for() in crates/codly-core/src/adapters/claude.rs for a Solo agent with a shell
TOOLS = "Bash,Read,Edit,Write,Grep,Glob,TaskCreate,TaskUpdate,TaskList,TaskGet,WebFetch,WebSearch,Agent,Monitor,TaskStop,Skill,LSP,ExitPlanMode"
CHANGED = "# Plan changed by the host\n\n1. Append the line `planet` (not `world`) to hello.txt.\n2. Read the file back.\n"
seen, state = [], {"exit_plan": 0}

def handler(req):
    seen.append(req)
    inp = req.get("input", {})
    if req["tool_name"] == "ExitPlanMode":
        state["exit_plan"] += 1
        if state["exit_plan"] == 1:
            return {"behavior": "deny", "message": "Not yet: add a final step that reads the file back to check it. Then present the plan again.", "interrupt": False}
        changed = dict(inp)
        changed["plan"] = CHANGED
        return {"behavior": "allow", "updatedInput": changed}
    return {"behavior": "allow", "updatedInput": inp}

s = Session("r4-plan-from-default", mode="manual", extra=["--tools", TOOLS], hard_timeout=280, on_can_use_tool=handler)
r = s.control("initialize", hooks=None)
print("current_permission_mode:", r and r["response"]["response"].get("current_permission_mode"))
print("set plan ->", (s.control("set_permission_mode", mode="plan") or {}).get("response"))
s.user("Plan a tiny change: add a second line `world` to hello.txt. Keep the plan to 2-3 short steps and present it with ExitPlanMode. When the plan is approved, carry out the approved plan.")
print("result:", s.wait_results(1, 220))
print("can_use_tool:", [(r["tool_name"], list(r.get("input", {}).keys())) for r in seen])
r = s.control("get_plan")
plan = (r or {}).get("response", {}).get("response", {})
path = plan.get("path")
s.note(hello_txt=open(os.path.join(s.cwd, "hello.txt")).read(), plan_file_exists=bool(path and os.path.exists(path)),
       plan_file_content=open(path).read() if path and os.path.exists(path) else None, exit_plan_mode_requests=state["exit_plan"])
print("hello.txt:", repr(open(os.path.join(s.cwd, "hello.txt")).read()))
print("plan file:", path and os.path.basename(path), repr(plan.get("content", "")[:200]))
print("cost:", s.cost(), "exit:", s.close())
