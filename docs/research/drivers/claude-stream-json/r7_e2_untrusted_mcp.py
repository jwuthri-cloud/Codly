"""E2: your own MCP servers in a folder you have not trusted, a second time: is the repo's `.mcp.json` server loaded?

The work folder's `.mcp.json` defines `ping-project` (mcp_ping_server.py; one line in started.txt per start). No run
passes --strict-mcp-config (dropping it is what the waiting code would do in such a folder).

argv[1]: user-login        --setting-sources user, the REAL login (part 4, section 8, `user-only`, again)
         user-local-login  --setting-sources user,local, the REAL login, and the work folder's
                           `.claude/settings.local.json` turns the project's servers on (part 4, `user-local-file`, again)
         local-scope-user  a THROW-AWAY HOME (no login) in which `claude mcp add --scope local` added `ping-local` for
                           this folder (kept in HOME/.claude.json under the folder's path); --setting-sources user
         local-scope-user-local    the same with --setting-sources user,local
         approval-user     a THROW-AWAY HOME whose HOME/.claude.json keeps, under this folder's path, an approval of the
                           project's server (`enabledMcpjsonServers: ["ping-project"]`, `hasTrustDialogAccepted: true`);
                           --setting-sources user
         approval-user-local       the same with --setting-sources user,local
argv[2]: a label (1, 2: the same scenario again), added to the fixture's name

The approval: the CLI writes an entry for each folder it runs in (part 5, section 1, read it from a throw-away HOME).
This script first lets the CLI write that entry (`claude mcp add --scope local`, then `remove`, throw-away HOME:
the entry stays), checks that the entry has the keys it sets, and only then changes their values. It invents no key.

THE LOGGED-IN RUNS READ YOUR OWN USER SETTINGS, as in part 4 (r6_e8_project_mcp_login.py): `"disableAllHooks":true` in
--settings, the claude.ai connectors off, the driver allows ONE tool (`mcp__ping-project__ping`) and denies every other
request, no mcp_status is sent, the sanitizer drops every server that is not the scenario's own and the notes say only
whether there were any. Your own stdio servers are still started by the CLI, as in any session of yours.
"""
import json, os, sys, time
from drv import Session, make_repo
from r5_lib import lines_of, next_turn, ping_server, stdio_server
from r7_lib import claude_cli, home, project_entry
variant = sys.argv[1] if len(sys.argv) > 1 else "local-scope-user"
name = "r7-untrusted-mcp-" + variant + ("-" + sys.argv[2] if len(sys.argv) > 2 else "")
cwd = make_repo(name)
ping_server(cwd)
marker, local_marker = os.path.join(cwd, "started.txt"), os.path.join(cwd, "started-local.txt")
for p in (marker, local_marker):
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
sources = "user,local" if variant.endswith("user-local") or variant == "user-local-login" else "user"
setup = {}
if variant.endswith("login"):
    if variant == "user-local-login":
        local_file = {"enableAllProjectMcpServers": True, "enabledMcpjsonServers": ["ping-project"]}
        os.makedirs(os.path.join(cwd, ".claude"), exist_ok=True)
        with open(os.path.join(cwd, ".claude", "settings.local.json"), "w") as f:
            json.dump(local_file, f)
        setup["settings_local_json_of_the_work_folder"] = local_file
    s = Session(name, extra=["--tools", "Bash", "--settings", json.dumps({"disableAllHooks": True, "disableClaudeAiConnectors": True})],
                setting_sources=sources, env={"ENABLE_CLAUDEAI_MCP_SERVERS": "false"}, **kw)
else:
    h = home(name)
    if variant.startswith("local-scope"):
        r = claude_cli(["mcp", "add", "--scope", "local", "ping-local", "--", "python3", "mcp_ping_server.py", "started-local.txt", "label=local"], cwd, {"HOME": h})
        setup["claude_mcp_add"] = dict(exit_code=r.returncode, first_line_of_its_output=r.stdout.strip().splitlines()[0] if r.stdout.strip() else "")
    else:
        # `claude mcp add --scope local` makes the CLI write the folder's entry in HOME/.claude.json; `remove` takes the
        # server out again and leaves the entry with its other keys.
        claude_cli(["mcp", "add", "--scope", "local", "ping-local", "--", "python3", "mcp_ping_server.py", "started-local.txt"], cwd, {"HOME": h})
        r = claude_cli(["mcp", "remove", "--scope", "local", "ping-local"], cwd, {"HOME": h})
        entry = project_entry(h, cwd)
        if entry is None or "enabledMcpjsonServers" not in entry or "hasTrustDialogAccepted" not in entry:
            raise SystemExit("the CLI wrote no entry with these keys for the folder: nothing is changed")
        cfg_path = os.path.join(h, ".claude.json")
        cfg = json.load(open(cfg_path))
        key = cwd if cwd in cfg["projects"] else os.path.realpath(cwd)
        cfg["projects"][key].update(enabledMcpjsonServers=["ping-project"], hasTrustDialogAccepted=True)
        with open(cfg_path, "w") as f:
            json.dump(cfg, f, indent=2)
        setup["claude_mcp_remove_exit_code"] = r.returncode
    setup["servers_started_by_the_setup_command"] = dict(project=len(lines_of(marker)), local=len(lines_of(local_marker)))
    for p in (marker, local_marker):  # count from the session's start on
        if os.path.exists(p):
            os.remove(p)
    entry = project_entry(h, cwd) or {}
    setup["project_entry_in_home_claude_json"] = {k: entry.get(k) for k in ("mcpServers", "enabledMcpjsonServers", "disabledMcpjsonServers", "hasTrustDialogAccepted")}
    s = Session(name, extra=["--tools", "Bash"], setting_sources=sources, env={"HOME": h}, **kw)

time.sleep(3.0)
s.note(setup=setup, before_first_message=True, servers_started=dict(project=len(lines_of(marker)), local=len(lines_of(local_marker))))
print("turn:", next_turn(s, "If you have a tool named exactly `%s`, call it once and reply with exactly what it returned. "
                            "If you have no tool with exactly that name, reply with the single word: none" % TOOL, 90))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
mine = [x for x in init.get("mcp_servers", []) if x.get("name") in ("ping-project", "ping-local")]
with s.cv:
    r = [e for e in s.events if e.get("type") == "result"]
s.note(server_starts=dict(project=len(lines_of(marker)), local=len(lines_of(local_marker))), init_entries_for_the_scenario_servers=mine,
       tools_of_the_scenario_servers=[t for t in init.get("tools", []) if t.startswith("mcp__ping-")],
       servers_of_the_users_own_setup_in_init=len(init.get("mcp_servers", [])) > len(mine),
       can_use_tool_requests=[[TOOL, q.get("mcp_server"), "allowed"] if q.get("tool_name") == TOOL else ["(another tool)", None, "denied"] for q in seen],
       result=r[-1].get("result") if r else None)
print("starts:", len(lines_of(marker)), len(lines_of(local_marker)), "init:", mine, "others:", len(init.get("mcp_servers", [])) > len(mine), "result:", r[-1].get("result") if r else None)
print("cost:", s.cost(), "exit:", s.close())
