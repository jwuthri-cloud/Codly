"""Scenario 2: control requests on an idle session and during a turn."""
import time
from drv import Session
s = Session("control-requests", extra=["--tools", "Bash,Read"], hard_timeout=240)

def asst_model(start):
    ev = s.wait_for(lambda e: e.get("type") == "assistant", 60, start)
    return ev and ev["message"]["model"]

# --- A. idle, before the first user message
s.control("initialize", hooks=None)
s.control("interrupt")
s.control("set_permission_mode", mode="plan")
s.control("set_permission_mode", mode="acceptEdits")
s.control("set_permission_mode", mode="bypassPermissions")
s.control("set_permission_mode", mode="acceptEdits")
s.control("set_permission_mode", mode="not-a-mode")
s.control("set_model", model="sonnet")
m = s.mark()
s.user("Reply with the single word: one")
print("after set_model sonnet:", asst_model(m)); s.wait_results(1, 60)
s.control("set_model", model="haiku")
m = s.mark()
s.user("Reply with the single word: two")
print("after set_model haiku:", asst_model(m)); s.wait_results(2, 60)
s.control("set_model", model="no-such-model-xyz")
s.control("set_model", model="haiku")
# read-only / harmless subtypes
for sub, fields in [
    ("get_context_usage", {}),
    ("mcp_status", {}),
    ("get_session_cost", {}),
    ("get_usage", {}),
    ("list_models", {}),
    ("get_status", {}),
    ("get_settings", {}),
    ("list_permission_rules", {}),
    ("get_plan", {}),
    ("get_hooks_listing", {}),
    ("get_binary_version", {}),
    ("file_suggestions", {"query": "READ"}),
    ("get_workspace_diff", {}),
    ("read_file", {"path": "hello.txt"}),
    ("list_directory", {"path": "."}),
    ("set_max_thinking_tokens", {"max_thinking_tokens": 1024}),
    ("set_max_thinking_tokens", {"max_thinking_tokens": None}),
    ("rename_session", {"title": "renamed-by-host"}),
    ("stop_task", {"task_id": "no-such-task"}),
    ("get_task_output", {"task_id": "no-such-task"}),
    ("cancel_async_message", {"message_uuid": "00000000-0000-4000-8000-000000000000"}),
    ("background_tasks", {}),
    ("rewind_files", {"user_message_id": "00000000-0000-4000-8000-000000000000", "dry_run": True}),
    ("definitely_not_a_subtype", {}),
]:
    r = s.control(sub, **fields)
    print(sub, "->", (r or {}).get("response", {}).get("subtype") if r else "NO RESPONSE")

# --- B. during a turn
m = s.mark()
s.user("Run the Bash command `sleep 20` and then reply with the single word: three")
s.wait_for(lambda e: e.get("type") == "assistant" and any(b.get("type") == "tool_use" for b in e["message"]["content"]), 60, m)
time.sleep(1.5)
s.control("get_context_usage")
s.control("set_permission_mode", mode="plan")
s.control("set_permission_mode", mode="acceptEdits")
s.control("set_model", model="sonnet")
s.control("set_model", model="haiku")
s.control("mcp_status")
t = time.time()
s.control("interrupt")
print("3 results:", s.wait_results(3, 30), "after %.1fs" % (time.time() - t))
time.sleep(1)
# alive after the interrupt?
m = s.mark()
s.user("Reply with the single word: four")
print("after interrupt:", asst_model(m)); s.wait_results(4, 60)
r = s.control("end_session")
print("end_session ->", r and r.get("response"))
time.sleep(3)
print("alive after end_session:", s.alive())
print("exit:", s.close())
