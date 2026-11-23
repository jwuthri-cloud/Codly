"""C13: set_model "default", apply_flag_settings with an effort level, and what --fallback-model shows.

No turn runs on another model than haiku: set_model default is followed by set_model haiku before the first turn.
"""
from drv import Session
s = Session("r4-model-effort-fallback", extra=["--tools", "Bash", "--fallback-model", "sonnet"], hard_timeout=150)
def ask(sub, **f):
    r = s.control(sub, **f)
    resp = (r or {}).get("response", {})
    print("%-22s %-40s -> %s" % (sub, f, str({k: v for k, v in resp.items() if k != "request_id"})[:400]))
    return resp
r = s.control("initialize", hooks=None)
ask("get_status")
ask("get_settings")
ask("set_model", model="default")
ask("get_settings")
ask("set_model", model="haiku")
ask("apply_flag_settings", settings={"effortLevel": "low"})
ask("get_settings")
ask("apply_flag_settings", settings={"effortLevel": "not-a-level"})
ask("get_settings")
ask("apply_flag_settings", settings={"effortLevel": "high"})
ask("get_settings")
s.user("Reply with the single word: pong")
print("result:", s.wait_results(1, 60))
ask("get_settings")
ask("apply_flag_settings", settings={"effortLevel": None})
ask("get_settings")
ask("set_model", model="default")
ask("set_model", model="haiku")
print("cost:", s.cost(), "exit:", s.close())
