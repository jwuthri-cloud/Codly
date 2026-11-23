"""E7: `mcp_reconnect` on a server that is CONNECTED, and a stdio server whose process is killed mid-session.

The server `ping-live` (mcp_ping_server.py) writes one line with its pid to started.txt each time it starts.
Order: mcp_status -> mcp_reconnect (the server is connected) -> mcp_status -> a turn that calls the tool ->
the driver kills the server process (SIGKILL; only a process whose command line is this server's) -> 8 s of reading ->
[the variant] -> mcp_reconnect -> mcp_status -> a turn that calls the tool.
After every mcp_status answer the driver notes again how often the server was started, so that a start caused by
the request itself shows.

argv[1]: status-first  after the kill: mcp_status, then a turn that calls the tool, then mcp_status
         call-first    after the kill: a turn that calls the tool, then mcp_status
"""
import json, os, re, sys, time
from drv import Session, make_repo, OUT
from r5_lib import allow_all, ask, lines_of, next_turn, ping_server, stdio_server
from r6_lib import command_of, is_alive, kinds, signal
variant = sys.argv[1] if len(sys.argv) > 1 else "status-first"
name = "r6-mcp-live-server-" + variant
cwd = make_repo(name)
ping_server(cwd)
marker = os.path.join(cwd, "started.txt")
if os.path.exists(marker):
    os.remove(marker)
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-live": stdio_server("started.txt")}}, f)
seen = []
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=200, on_can_use_tool=allow_all(seen))
CALL = "Call the tool mcp__ping-live__ping once and reply with exactly what it returned. If the call fails, reply with the error text. Do not try again."


def pids():
    return [re.search(r"pid=(\d+)", l).group(1) for l in lines_of(marker)]


def state(label):
    """A note (how often the server was started, which of those processes live) and then mcp_status."""
    p = pids()
    s.note(after=label, server_starts_so_far=len(p), server_processes_alive=[is_alive(x) for x in p])
    r = ask(s, "mcp_status").get("response", {})
    time.sleep(0.5)
    p = pids()
    s.note(after_the_mcp_status_answer=True, server_starts_so_far=len(p), server_processes_alive=[is_alive(x) for x in p])
    return [x.get("status") for x in r.get("mcpServers", []) if x.get("name") == "ping-live"]


def call(label):
    m = s.mark()
    ok = next_turn(s, CALL, 60)
    with s.cv:
        ev = s.events[m:]
    init = [e for e in ev if e.get("type") == "system" and e.get("subtype") == "init"]
    res = [json.dumps(b.get("content"))[:300] for e in ev if e.get("type") == "user" and isinstance(e["message"]["content"], list)
           for b in e["message"]["content"] if b.get("type") == "tool_result"]
    s.note(turn=label, ok=ok, init_mcp_servers=init[0].get("mcp_servers") if init else None,
           tool_in_init_tools="mcp__ping-live__ping" in (init[0].get("tools", []) if init else []), tool_results=res, server_starts_so_far=len(pids()))
    print("turn (%s): ok=%s init=%s tool results=%s starts=%d" % (label, ok, init[0].get("mcp_servers") if init else None, res, len(pids())))


time.sleep(3.0)
print("status:", state("start + 3 s"))
ask(s, "mcp_reconnect", wait=30, serverName="ping-live")
print("status:", state("mcp_reconnect on the connected server"))
call("after the reconnect")

victim = pids()[-1]
ours = "mcp_ping_server.py started.txt" in command_of(victim)
if not ours:
    print("exit:", s.close())
    raise SystemExit("pid %s is not this scenario's server: nothing was killed" % victim)
m = s.mark()
s.note(killed=dict(signal="SIGKILL", to="the server process (the last pid in started.txt)", delivered=signal(victim, "KILL")))
time.sleep(8.0)
s.note(stdout_lines_in_the_8s_after_the_kill=kinds(s, m), server_process_alive=is_alive(victim))
print("lines in the 8 s after the kill:", kinds(s, m))
if variant == "status-first":
    print("status:", state("the server process was killed 8 s ago"))
call("after the kill")
print("status:", state("the turn after the kill"))
ask(s, "mcp_reconnect", wait=30, serverName="ping-live")
print("status:", state("mcp_reconnect after the kill"))
call("after the second reconnect")
print("cost:", s.cost(), "exit:", s.close())
print("server processes left:", [x for x in pids() if is_alive(x)])
