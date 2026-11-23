"""E19: a work dir with a .mcp.json (a harmless stdio server) and --setting-sources project.

argv[1]: project  (no --strict-mcp-config: does the headless CLI start the server without any approval?)
         strict   (the same folder with --strict-mcp-config: control)
The server (mcp_ping_server.py, copied into the work dir) writes a line to started.txt when it runs.
No mcp_status is sent in the `project` run: without --strict-mcp-config the answer could list the user's own servers
with their configuration. The init line only has names and states, and the sanitizer keeps only this scenario's server.
"""
import json, os, shutil, sys, time
from drv import Session, make_repo
variant = sys.argv[1] if len(sys.argv) > 1 else "project"
cwd = make_repo("r4-project-mcp-" + variant)
here = os.path.dirname(os.path.abspath(__file__))
shutil.copy(os.path.join(here, "mcp_ping_server.py"), os.path.join(cwd, "mcp_ping_server.py"))
marker = os.path.join(cwd, "started.txt")
if os.path.exists(marker):
    os.remove(marker)
with open(os.path.join(cwd, ".mcp.json"), "w") as f:
    json.dump({"mcpServers": {"ping-project": {"type": "stdio", "command": "python3", "args": ["mcp_ping_server.py", "started.txt"]}}}, f)
seen = []

def handler(req):
    seen.append(req)
    return {"behavior": "allow", "updatedInput": req.get("input", {})}

s = Session("r4-project-mcp-" + variant, mode="manual", extra=["--tools", "Bash"], hard_timeout=120, on_can_use_tool=handler,
            strict_mcp=(variant == "strict"), setting_sources="project")
time.sleep(3.0)
s.note(before_first_message=True, server_started=os.path.exists(marker))
s.user("If you have a tool whose name ends with `ping`, call it once and reply with what it returned. If you have no such tool, reply with the single word: none")
print("result:", s.wait_results(1, 90))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5)
mine = [x for x in (init or {}).get("mcp_servers", []) if x.get("name") == "ping-project"]
others = len((init or {}).get("mcp_servers", [])) - len(mine)
s.note(server_started=os.path.exists(marker), marker=open(marker).read().count("\n") if os.path.exists(marker) else 0,
       init_entry_for_this_server=mine, servers_of_the_users_own_setup_in_init=others > 0,
       can_use_tool_requests=[(r["tool_name"], r.get("decision_reason_type")) for r in seen])
print("started:", os.path.exists(marker), "init entry:", mine, "other servers in init:", others, "requests:", [(r["tool_name"]) for r in seen])
print("cost:", s.cost(), "exit:", s.close())
