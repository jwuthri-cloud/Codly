"""E8: a project `.mcp.json` server with the app's flags and the REAL login: is it loaded, can its tool be called?

The work folder's `.mcp.json` defines `ping-project` (mcp_ping_server.py; one line in started.txt per start).
No run passes --strict-mcp-config.

argv[1]: enable-all-false  --setting-sources user,project, --settings '{"enableAllProjectMcpServers":false}'
         enabled-other     --setting-sources user,project, --settings '{"enabledMcpjsonServers":["another-server"]}'
         disabled          --setting-sources user,project, --settings '{"disabledMcpjsonServers":["ping-project"]}'
         user-only         --setting-sources user (the project's settings are not a source), nothing in --settings
         user-only-home    the same with a THROW-AWAY HOME (no login: it shows what is loaded, not a tool call)
         user-local-home   --setting-sources user,local with a THROW-AWAY HOME (no login)
         hooks-on, hooks-off   a check of this script's own safety, with a THROW-AWAY HOME whose settings.json has two
                           harmless hooks (SessionStart, UserPromptSubmit: each appends a line to a file in the work
                           folder): `hooks-off` adds `"disableAllHooks":true` to --settings, as the logged-in runs do

THE LOGGED-IN RUNS (the first four) READ YOUR OWN USER SETTINGS: `--setting-sources user` is the question. So that they
change nothing of yours:
- `"disableAllHooks":true` is in --settings: your hooks do not run (checked with `hooks-on` / `hooks-off` first);
- the claude.ai connectors are off (setting + environment variable), as in r5_e11 and r5_e12;
- the driver allows ONE tool, `mcp__ping-project__ping`, and denies every other request, so no tool of your own
  servers can run. Your own stdio servers are still started by the CLI, as in any session of yours;
- no mcp_status is sent (its answer lists server configurations); the sanitizer drops every server that is not the
  scenario's own; the notes say only whether there were any.
"""
import json, os, sys, time
from drv import Session, make_repo, OUT
from r5_lib import lines_of, next_turn, ping_server, stdio_server
variant = sys.argv[1] if len(sys.argv) > 1 else "user-only-home"
name = "r6-project-mcp-" + variant
cwd = make_repo(name)
ping_server(cwd)
marker, hook_file = os.path.join(cwd, "started.txt"), os.path.join(cwd, "hook-ran.txt")
for p in (marker, hook_file):
    if os.path.exists(p):
        os.remove(p)
with open(os.path.join(cwd, ".mcp.json"), "w") as f:
    json.dump({"mcpServers": {"ping-project": stdio_server("started.txt")}}, f)
TOOL = "mcp__ping-project__ping"
seen = []


def handler(req):
    seen.append(req)
    if req.get("tool_name") == TOOL:
        return {"behavior": "allow", "updatedInput": req.get("input", {})}
    return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}


kw = dict(mode="manual", hard_timeout=120, on_can_use_tool=handler, strict_mcp=False)
flag = {"enable-all-false": {"enableAllProjectMcpServers": False}, "enabled-other": {"enabledMcpjsonServers": ["another-server"]},
        "disabled": {"disabledMcpjsonServers": ["ping-project"]}}.get(variant, {})
login = variant in ("enable-all-false", "enabled-other", "disabled", "user-only")
if login:
    flag.update(disableAllHooks=True, disableClaudeAiConnectors=True)
    s = Session(name, extra=["--tools", "Bash", "--settings", json.dumps(flag)], setting_sources="user" if variant == "user-only" else "user,project",
                env={"ENABLE_CLAUDEAI_MCP_SERVERS": "false"}, **kw)
else:
    home = os.path.join(OUT, "work", name + "-home")
    os.makedirs(os.path.join(home, ".claude"), exist_ok=True)
    if variant.startswith("hooks"):
        hook = {"hooks": [{"type": "command", "command": "echo ran >> " + hook_file}]}
        with open(os.path.join(home, ".claude", "settings.json"), "w") as f:
            json.dump({"hooks": {"SessionStart": [hook], "UserPromptSubmit": [hook]}}, f)
        if variant == "hooks-off":
            flag = {"disableAllHooks": True}
    s = Session(name, extra=["--tools", "Bash"] + (["--settings", json.dumps(flag)] if flag else []),
                setting_sources="user,local" if variant == "user-local-home" else "user", env={"HOME": home}, **kw)

time.sleep(3.0)
s.note(before_first_message=True, server_started=os.path.exists(marker))
print("turn:", next_turn(s, "If you have a tool named exactly `%s`, call it once and reply with exactly what it returned. "
                            "If you have no tool with exactly that name, reply with the single word: none" % TOOL, 90))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
mine = [x for x in init.get("mcp_servers", []) if x.get("name") == "ping-project"]
with s.cv:
    r = [e for e in s.events if e.get("type") == "result"]
    hooks = sum(1 for e in s.events if e.get("type") == "system" and str(e.get("subtype", "")).startswith("hook"))
s.note(server_starts=len(lines_of(marker)), init_entry_for_this_server=mine, tool_in_init=TOOL in init.get("tools", []),
       servers_of_the_users_own_setup_in_init=len(init.get("mcp_servers", [])) > len(mine), hook_lines_in_the_stream=hooks,
       hook_file_lines=len(lines_of(hook_file)),
       can_use_tool_requests=[[TOOL, q.get("mcp_server"), "allowed"] if q.get("tool_name") == TOOL else ["(another tool)", None, "denied"] for q in seen],
       result=r[-1].get("result") if r else None)
print("starts:", len(lines_of(marker)), "init:", mine, "tool in init:", TOOL in init.get("tools", []), "others:", len(init.get("mcp_servers", [])) > len(mine))
print("hook lines:", hooks, "hook file lines:", len(lines_of(hook_file)), "requests:", [q.get("tool_name") for q in seen], "result:", (r[-1].get("result") if r else None))
print("cost:", s.cost(), "exit:", s.close())
