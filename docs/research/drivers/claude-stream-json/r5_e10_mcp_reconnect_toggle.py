"""E10: a SUCCESSFUL `mcp_reconnect`, and `mcp_toggle` off and on, on a working stdio server.

The server (`ping-flaky`: mcp_ping_server.py with need=switch.on) exits at once while the file switch.on is missing.
Order: mcp_status (failed) -> mcp_reconnect (still failing) -> the driver creates switch.on -> mcp_reconnect ->
mcp_status -> mcp_toggle off -> mcp_status -> mcp_toggle on -> mcp_status -> one turn that calls the tool.
"""
import json, os, time
from drv import Session, make_repo, OUT
from r5_lib import allow_all, ask, lines_of, next_turn, ping_server, stdio_server
name = "r5-mcp-reconnect-toggle"
cwd = make_repo(name)
ping_server(cwd)
marker, switch = os.path.join(cwd, "started.txt"), os.path.join(cwd, "switch.on")
for p in (marker, switch):
    if os.path.exists(p):
        os.remove(p)
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-flaky": stdio_server("started.txt", need="switch.on")}}, f)
seen = []
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=150, on_can_use_tool=allow_all(seen))

def step(label, sub=None, **f):
    if sub:
        ask(s, sub, wait=30, **f)
    s.note(after=label, server_starts_so_far=len(lines_of(marker)), switch_file_exists=os.path.exists(switch))
    ask(s, "mcp_status")

time.sleep(3.0)
step("start + 3 s")
step("mcp_reconnect while the server still fails", "mcp_reconnect", serverName="ping-flaky")
open(switch, "w").close()
step("mcp_reconnect after the switch file was made", "mcp_reconnect", serverName="ping-flaky")
step("mcp_toggle off", "mcp_toggle", serverName="ping-flaky", enabled=False)
step("mcp_toggle on", "mcp_toggle", serverName="ping-flaky", enabled=True)
print("turn:", next_turn(s, "Call the tool mcp__ping-flaky__ping once and reply with what it returned."))
init = [e for e in s.events if e.get("type") == "system" and e.get("subtype") == "init"][-1]
s.note(after="the turn", server_starts_so_far=len(lines_of(marker)), init_mcp_servers=init.get("mcp_servers"))
print("init mcp_servers:", init.get("mcp_servers"), "starts:", len(lines_of(marker)))
print("cost:", s.cost(), "exit:", s.close())
