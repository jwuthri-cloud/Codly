"""E8: a server that cannot start at the process's start, turned off, then on again in a NEW process (the saved switch:
part 5, section 1).

A THROW-AWAY HOME (a switch is saved in its ~/.claude.json): no login, no model request. The server `ping-flaky`
(mcp_ping_server.py with need=switch.on: it exits at once with code 1 while the file switch.on is missing; one line in
started.txt per start) comes with --mcp-config, as Codly passes it. switch.on is missing at first.

Process a (r8-mcp-cannot-start-saved-<label>-a): start + 3 s -> mcp_status -> mcp_toggle off -> mcp_status -> stdin closed.
Process b (…-b): a new process in the same folder: start + 3 s -> mcp_status -> mcp_toggle on (switch.on still
           missing) -> mcp_status -> switch.on is made -> mcp_reconnect -> mcp_status -> stdin closed.
Process c (…-c): a new process in the same folder (switch.on there): start + 3 s -> mcp_status -> stdin closed.
After each step a note: how often the server was started so far, whether switch.on exists, and what the throw-away
HOME's ~/.claude.json keeps for this folder in `disabledMcpServers`.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import ask, lines_of, ping_server, stdio_server
from r7_lib import home, project_entry
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
name = "r8-mcp-cannot-start-saved-" + LABEL
cwd = make_repo(name)
ping_server(cwd)
marker, switch = os.path.join(cwd, "started.txt"), os.path.join(cwd, "switch.on")
for p in (marker, switch):
    if os.path.exists(p):
        os.remove(p)
h = home(name)
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-flaky": stdio_server("started.txt", need="switch.on")}}, f)


def start(part):
    s = Session("%s-%s" % (name, part), extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=120, cwd=cwd, env={"HOME": h})
    time.sleep(3.0)
    step(s, "start + 3 s")
    return s


def step(s, label, sub=None, **f):
    if sub:
        ask(s, sub, wait=40, **f)
    time.sleep(1.0)
    s.note(after=label, server_starts_so_far=len(lines_of(marker)), switch_file_exists=os.path.exists(switch),
           saved_disabledMcpServers=(project_entry(h, cwd) or {}).get("disabledMcpServers"))
    r = ask(s, "mcp_status").get("response", {})
    print("%-40s -> %s" % (label, [(x.get("name"), x.get("status"), x.get("error")) for x in r.get("mcpServers", [])]))


a = start("a")
step(a, "mcp_toggle off", "mcp_toggle", serverName="ping-flaky", enabled=False)
a.close()
b = start("b")
step(b, "mcp_toggle on while the program exits at once", "mcp_toggle", serverName="ping-flaky", enabled=True)
open(switch, "w").close()
step(b, "mcp_reconnect after switch.on was made", "mcp_reconnect", serverName="ping-flaky")
b.close()
c = start("c")
print("exit:", c.close())
