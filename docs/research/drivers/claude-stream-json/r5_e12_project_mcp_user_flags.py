"""E12: a project `.mcp.json` server with the app's own flags (`--setting-sources user,project`, no
--strict-mcp-config): is it loaded headless without any approval, and which settings keep it out?

Every variant but the last uses a THROW-AWAY HOME (never the real one). A throw-away HOME has no login: the turn that
is sent ends with "Not logged in" and costs nothing, but its init line and mcp_status show what was loaded.

argv[1]: plain                  nothing else
         flag-disabled          --settings '{"disabledMcpjsonServers":["ping-project"]}'
         flag-enable-all-false  --settings '{"enableAllProjectMcpServers":false}'
         flag-enabled-other     --settings '{"enabledMcpjsonServers":["another-server"]}'
         user-disabled          the same key as flag-disabled, in ~/.claude/settings.json of the throw-away HOME
         login-flag-disabled    real login, --setting-sources project, the flag of flag-disabled, claude.ai connectors
                                turned off, no mcp_status (as r4_e19: the sanitizer drops servers of your own setup)
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import allow_all, lines_of, next_turn, ping_server, stdio_server
variant = sys.argv[1] if len(sys.argv) > 1 else "plain"
name = "r5-project-mcp-" + variant
cwd = make_repo(name)
ping_server(cwd)
marker = os.path.join(cwd, "started.txt")
if os.path.exists(marker):
    os.remove(marker)
with open(os.path.join(cwd, ".mcp.json"), "w") as f:
    json.dump({"mcpServers": {"ping-project": stdio_server("started.txt")}}, f)
flag = {"flag-disabled": {"disabledMcpjsonServers": ["ping-project"]}, "login-flag-disabled": {"disabledMcpjsonServers": ["ping-project"]},
        "flag-enable-all-false": {"enableAllProjectMcpServers": False}, "flag-enabled-other": {"enabledMcpjsonServers": ["another-server"]}}.get(variant, {})
seen = []
kw = dict(mode="manual", hard_timeout=120, on_can_use_tool=allow_all(seen), strict_mcp=False)
if variant.startswith("login"):
    flag["disableClaudeAiConnectors"] = True
    s = Session(name, extra=["--tools", "Bash", "--settings", json.dumps(flag)], setting_sources="project",
                env={"ENABLE_CLAUDEAI_MCP_SERVERS": "false"}, **kw)
else:
    home = os.path.join(OUT, "work", name + "-home")
    os.makedirs(os.path.join(home, ".claude"), exist_ok=True)
    if variant == "user-disabled":
        with open(os.path.join(home, ".claude", "settings.json"), "w") as f:
            json.dump({"disabledMcpjsonServers": ["ping-project"]}, f)
    s = Session(name, extra=["--tools", "Bash"] + (["--settings", json.dumps(flag)] if flag else []), setting_sources="user,project",
                env={"HOME": home}, **kw)
time.sleep(3.0)
s.note(before_first_message=True, server_started=os.path.exists(marker))
if not variant.startswith("login"):
    s.control("mcp_status")
print("turn:", next_turn(s, "If you have a tool whose name ends with `ping`, call it once and reply with what it returned. "
                            "If you have no such tool, reply with the single word: none"))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
mine = [x for x in init.get("mcp_servers", []) if x.get("name") == "ping-project"]
with s.cv:
    r = [e for e in s.events if e.get("type") == "result"]
s.note(server_starts=len(lines_of(marker)), init_entry_for_this_server=mine,
       tool_in_init="mcp__ping-project__ping" in init.get("tools", []),
       other_servers_in_init=len(init.get("mcp_servers", [])) - len(mine), result=r[-1].get("result") if r else None)
print("starts:", len(lines_of(marker)), "init:", mine, "others:", len(init.get("mcp_servers", [])) - len(mine),
      "result:", (r[-1].get("result") if r else None))
print("cost:", s.cost(), "exit:", s.close())
