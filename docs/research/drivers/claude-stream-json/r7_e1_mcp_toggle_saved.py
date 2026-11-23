"""E1: `mcp_toggle` off, the process exits, then a NEW process in the same folder (once fresh, once with --resume):
what does `mcp_status` say about that server? Is the switch saved, and where?

A THROW-AWAY HOME (a switch that is saved must not land in your own ~/.claude.json): no login, so no model request.
Each process gets the server `ping-live` (mcp_ping_server.py, one line in started.txt per start) with --mcp-config,
`--strict-mcp-config --setting-sources local`, as Codly starts it.

Process a (r7-mcp-toggle-saved-<label>-a): mcp_status -> mcp_toggle off -> mcp_status -> one message (it ends at once
           with "Not logged in"; the CLI writes the session's transcript, so that it can be resumed) -> stdin closed.
Process b (…-b): a new session in the same folder: mcp_status -> one message (its `init` line) -> stdin closed.
Process c (…-c): --resume of a's session in the same folder: mcp_status -> one message -> stdin closed. If the server
           is still off here, `mcp_toggle` on -> mcp_status.
After each step a note names the files under the throw-away HOME and the work folder that are new or changed since
the step before (transcripts and caches left out: r7_lib.files_of), the folder's entry in HOME/.claude.json (only its
MCP keys), and how often the server was started.

argv[1]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import ask, lines_of, next_turn, ping_server, stdio_server
from r6_lib import is_init
from r7_lib import changed, files_of, home, project_entry
run = sys.argv[1] if len(sys.argv) > 1 else "1"
name = "r7-mcp-toggle-saved-" + run
cwd = make_repo(name)
ping_server(cwd)
marker = os.path.join(cwd, "started.txt")
if os.path.exists(marker):
    os.remove(marker)
h = home(name)
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-live": stdio_server("started.txt")}}, f)
seen = {"files": files_of(h, cwd)}


def mcp_keys(entry):
    return None if entry is None else {k: v for k, v in entry.items() if "mcp" in k.lower() or "Mcp" in k}


def state(s, label):
    now = files_of(h, cwd)
    s.note(after=label, files_changed=changed(seen["files"], now, os.path.dirname(h)), project_entry_mcp_keys=mcp_keys(project_entry(h, cwd)),
           server_starts_so_far=len(lines_of(marker)))
    seen["files"] = now


def status(s):
    r = ask(s, "mcp_status").get("response", {})
    return [x.get("status") for x in r.get("mcpServers", []) if x.get("name") == "ping-live"]


def start(label, extra=()):
    s = Session("%s-%s" % (name, label), extra=["--tools", "Bash", "--mcp-config", cfg] + list(extra), session_id=None if extra else "new",
                hard_timeout=90, cwd=cwd, env={"HOME": h})
    time.sleep(3.0)
    state(s, "start + 3 s")
    return s


def finish(s):
    m = s.mark()
    next_turn(s, "Reply with the single word: pong", 30)
    init = s.wait_for(is_init, 2, m) or {}
    s.note(init_mcp_servers=init.get("mcp_servers"), tool_in_init="mcp__ping-live__ping" in init.get("tools", []))
    code = s.close()
    state_after = files_of(h, cwd)
    print("init:", init.get("mcp_servers"), "exit:", code, "files changed at the end:", changed(seen["files"], state_after, os.path.dirname(h)))
    seen["files"] = state_after


a = start("a")
print("a status:", status(a))
ask(a, "mcp_toggle", serverName="ping-live", enabled=False)
state(a, "mcp_toggle off")
print("a status after off:", status(a))
sid = a.sid
finish(a)
b = start("b")
print("b status:", status(b))
finish(b)
c = start("c", ["--resume", sid])
st = status(c)
print("c status:", st)
if st and st[0] == "disabled":
    ask(c, "mcp_toggle", serverName="ping-live", enabled=True)
    state(c, "mcp_toggle on")
    print("c status after on:", status(c))
finish(c)
