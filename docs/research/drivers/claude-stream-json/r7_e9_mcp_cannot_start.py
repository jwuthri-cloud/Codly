"""E9: "Turn on" (`mcp_toggle` enabled) of a server whose program cannot start, and `mcp_reconnect` on a server whose
program is gone: the answer (success or error, which words) and the status after it.

A THROW-AWAY HOME (a switch is saved in ~/.claude.json: part 5, section 1): no login, no model request.
The server `ping-flaky` (mcp_ping_server.py with need=switch.on: it exits at once with code 1 while the file switch.on
is missing; one line in started.txt per start) comes with --mcp-config, as Codly passes it.
Order: switch.on exists -> mcp_status (connected) -> mcp_toggle off -> switch.on is deleted -> mcp_toggle on (the
program starts and exits at once) -> mcp_status -> switch.on again -> mcp_reconnect -> mcp_status (connected) ->
mcp_ping_server.py is deleted (the program is gone: python3 cannot open it) -> mcp_reconnect -> mcp_status ->
mcp_toggle off -> mcp_toggle on (still gone) -> mcp_status.
A second server, `no-such-command`, names a program that does not exist: mcp_toggle off -> mcp_toggle on ->
mcp_reconnect, each followed by mcp_status.
After each step a note says how often `ping-flaky` was started, which files exist, and what the throw-away HOME's
~/.claude.json keeps for this folder in `disabledMcpServers` (where a switch is saved).

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import ask, lines_of, ping_server, stdio_server
from r7_lib import home, project_entry
run = sys.argv[1] if len(sys.argv) > 1 else "1"
name = "r7-mcp-cannot-start-" + run
cwd = make_repo(name)
program = ping_server(cwd)
marker, switch = os.path.join(cwd, "started.txt"), os.path.join(cwd, "switch.on")
if os.path.exists(marker):
    os.remove(marker)
open(switch, "w").close()
h = home(name)
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-flaky": stdio_server("started.txt", need="switch.on"),
                              "no-such-command": {"type": "stdio", "command": "codly-no-such-program-7", "args": []}}}, f)
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=150, env={"HOME": h})


def step(label, sub=None, **f):
    if sub:
        ask(s, sub, wait=40, **f)
    time.sleep(1.0)
    s.note(after=label, server_starts_so_far=len(lines_of(marker)), switch_file_exists=os.path.exists(switch), program_file_exists=os.path.exists(program),
           saved_disabledMcpServers=(project_entry(h, cwd) or {}).get("disabledMcpServers"))
    r = ask(s, "mcp_status").get("response", {})
    print("%-50s -> %s" % (label, [(x.get("name"), x.get("status"), x.get("error")) for x in r.get("mcpServers", [])]))


time.sleep(3.0)
step("start + 3 s")
step("mcp_toggle off", "mcp_toggle", serverName="ping-flaky", enabled=False)
os.remove(switch)
step("mcp_toggle on while the program exits at once", "mcp_toggle", serverName="ping-flaky", enabled=True)
open(switch, "w").close()
step("mcp_reconnect after switch.on was made again", "mcp_reconnect", serverName="ping-flaky")
os.remove(program)
step("mcp_reconnect after the program file was deleted", "mcp_reconnect", serverName="ping-flaky")
step("mcp_toggle off", "mcp_toggle", serverName="ping-flaky", enabled=False)
step("mcp_toggle on while the program file is gone", "mcp_toggle", serverName="ping-flaky", enabled=True)
step("mcp_toggle off (no-such-command)", "mcp_toggle", serverName="no-such-command", enabled=False)
step("mcp_toggle on (no-such-command)", "mcp_toggle", serverName="no-such-command", enabled=True)
step("mcp_reconnect (no-such-command)", "mcp_reconnect", serverName="no-such-command")
print("exit:", s.close())
