"""E11: the same server name in `--mcp-config` and in the USER's own setup. Which one is loaded, and what do the init
line and `mcp_status` say about `source`?

A THROW-AWAY HOME (never the real one) whose `.claude.json` defines `ping-same` (answers "pong from user", writes
started-user.txt); `--mcp-config` defines `ping-same` too ("pong from config", started-config.txt).
A throw-away HOME has no login, so no tool call can be made: the run records what is loaded, not a request.

argv[1]: clash   --setting-sources user, no --strict-mcp-config (both definitions could load)
         strict  the same with --strict-mcp-config (control)
argv[2]: a label for the recording (1, 2).
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import ask, lines_of, next_turn, ping_server, stdio_server
variant = sys.argv[1] if len(sys.argv) > 1 else "clash"
run = sys.argv[2] if len(sys.argv) > 2 else "1"
name = "r6-mcp-user-%s-%s" % (variant, run)
cwd = make_repo(name)
ping_server(cwd)
for f in os.listdir(cwd):
    if f.startswith("started"):
        os.remove(os.path.join(cwd, f))
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-same": stdio_server("started-config.txt", label="config")}}, f)
home = os.path.join(OUT, "work", name + "-home")
os.makedirs(home, exist_ok=True)
with open(os.path.join(home, ".claude.json"), "w") as f:
    json.dump({"mcpServers": {"ping-same": stdio_server("started-user.txt", label="user")}}, f)
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], mode="manual", hard_timeout=90, strict_mcp=variant == "strict", setting_sources="user",
            env={"HOME": home})


def starts():
    return {f: len(lines_of(os.path.join(cwd, f))) for f in sorted(os.listdir(cwd)) if f.startswith("started")}


time.sleep(3.0)
status = [x for x in ask(s, "mcp_status").get("response", {}).get("mcpServers", []) if x.get("name") == "ping-same"]
print("turn:", next_turn(s, "Call the tool mcp__ping-same__ping once and reply with exactly what it returned."))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
mine = [x for x in init.get("mcp_servers", []) if x.get("name") == "ping-same"]
with s.cv:
    r = [e for e in s.events if e.get("type") == "result"]
s.note(server_starts=starts(), init_entries_for_this_name=mine, other_servers_in_init=len(init.get("mcp_servers", [])) - len(mine),
       mcp_status_entries_for_this_name=[{k: x.get(k) for k in ("name", "status", "scope", "source")} for x in status],
       result=r[-1].get("result") if r else None)
print("starts:", starts(), "init:", mine, "mcp_status:", [{k: x.get(k) for k in ("name", "status", "scope", "source")} for x in status])
print("result:", r[-1].get("result") if r else None, "cost:", s.cost(), "exit:", s.close())
