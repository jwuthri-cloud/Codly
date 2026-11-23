"""E11: the `mcp_server` object (`name`, `source`) of a `can_use_tool` request, by where the server is defined.

argv[1]: config  a server from --mcp-config (with --strict-mcp-config, --setting-sources local)
         clash   --mcp-config AND the project's .mcp.json both define `ping-same` (answers "pong from config" /
                 "pong from project"); --setting-sources project, no --strict-mcp-config. Which one runs?
         user    a server from the user's own ~/.claude.json, with a THROW-AWAY HOME (never the real one). A throw-away
                 HOME has no login, so no tool call can be made: the run records mcp_status and the init line only.
(A server from the project's .mcp.json alone is part 2, scenario 19.)

`clash` runs without --strict-mcp-config with the real login, like r4_e19: the CLI may also load servers of your own
setup. The run turns the claude.ai connectors off (setting + environment variable), sends no mcp_status, and the
sanitizer drops every server that is not the scenario's own.
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import allow_all, lines_of, next_turn, ping_server, stdio_server
variant = sys.argv[1] if len(sys.argv) > 1 else "config"
name = "r5-mcp-source-" + variant
cwd = make_repo(name)
ping_server(cwd)
for f in os.listdir(cwd):
    if f.startswith("started"):
        os.remove(os.path.join(cwd, f))
cfg = os.path.join(OUT, "work", name + ".json")
seen = []
kw = dict(mode="manual", hard_timeout=120, on_can_use_tool=allow_all(seen))
server = {"config": "ping-config", "clash": "ping-same", "user": "ping-user"}[variant]

if variant == "config":
    with open(cfg, "w") as f:
        json.dump({"mcpServers": {server: stdio_server("started-config.txt", label="config")}}, f)
    s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], **kw)
elif variant == "clash":
    with open(cfg, "w") as f:
        json.dump({"mcpServers": {server: stdio_server("started-config.txt", label="config")}}, f)
    with open(os.path.join(cwd, ".mcp.json"), "w") as f:
        json.dump({"mcpServers": {server: stdio_server("started-project.txt", label="project")}}, f)
    s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg, "--settings", json.dumps({"disableClaudeAiConnectors": True})],
                strict_mcp=False, setting_sources="project", env={"ENABLE_CLAUDEAI_MCP_SERVERS": "false"}, **kw)
else:
    home = os.path.join(OUT, "work", name + "-home")
    os.makedirs(home, exist_ok=True)
    with open(os.path.join(home, ".claude.json"), "w") as f:
        json.dump({"mcpServers": {server: stdio_server("started-user.txt", label="user")}}, f)
    s = Session(name, extra=["--tools", "Bash"], strict_mcp=False, setting_sources="user", env={"HOME": home}, **kw)

time.sleep(3.0)
if variant == "user":
    s.control("mcp_status")
print("turn:", next_turn(s, "Call the tool mcp__%s__ping once and reply with exactly what it returned." % server))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
mine = [x for x in init.get("mcp_servers", []) if x.get("name") == server]
reqs = [{"tool_name": r.get("tool_name"), "mcp_server": r.get("mcp_server")} for r in seen]
with s.cv:
    r = [e for e in s.events if e.get("type") == "result"]
s.note(server_starts={f: len(lines_of(os.path.join(cwd, f))) for f in sorted(os.listdir(cwd)) if f.startswith("started")},
       init_entries_for_this_server=mine, other_servers_in_init=len(init.get("mcp_servers", [])) - len(mine),
       can_use_tool_requests=reqs, result=r[-1].get("result") if r else None)
print("starts:", {f: len(lines_of(os.path.join(cwd, f))) for f in sorted(os.listdir(cwd)) if f.startswith("started")})
print("init:", mine, "others in init:", len(init.get("mcp_servers", [])) - len(mine))
print("requests:", reqs, "result:", r[-1].get("result") if r else None)
print("cost:", s.cost(), "exit:", s.close())
