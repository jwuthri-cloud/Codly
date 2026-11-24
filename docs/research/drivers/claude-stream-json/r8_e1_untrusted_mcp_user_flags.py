"""E1: your own setup in a folder you have not trusted, the last two forms. No run passes --strict-mcp-config (dropping
it is what the waiting code does there); --setting-sources is `user` or `user,local`.

A THROW-AWAY HOME at every run (no login: the one message ends at once with "Not logged in"; the recording shows what
is LOADED and STARTED). The work folder's `.mcp.json` defines `ping-project` (one line in started.txt per start).

argv[1]: enable-all-<src>    HOME/.claude/settings.json (the USER settings file) has `"enableAllProjectMcpServers": true`
         enabled-named-<src> HOME/.claude/settings.json has `"enabledMcpjsonServers": ["ping-project"]`
         plugin-<src>        a plugin is installed for the user (`claude plugin marketplace add` of a local folder, then
                             `claude plugin install --scope user`, both with the throw-away HOME): `ping-plugin`, whose
                             `.mcp.json` brings the server `ping-plugged` (one line in started-plugin.txt per start)
         plugin-repo-<src>   the same plugin installed, then `claude plugin disable --scope user`; the work folder's
                             `.claude/settings.json` AND `.claude/settings.local.json` both say
                             `"enabledPlugins": {"ping-plugin@codly-test-market": true}` (files of the repo)
         plugin-repo-local-<src>    the same, with only `.claude/settings.local.json`
         plugin-repo-project-<src>  the same, with only `.claude/settings.json`
         <src> is `user` or `user-local` (--setting-sources user / user,local)
argv[2]: a label (1, 2: the same scenario again), added to the fixture's name

The driver allows ONE tool per server (`mcp__ping-project__ping`, the plugin's ping) and denies every other request
(none comes without a login). No mcp_status is sent: init says what was loaded.
"""
import os, sys, time
from drv import Session, make_repo
from r5_lib import lines_of, next_turn, ping_server, stdio_server
from r7_lib import home, read_json
from r8_lib import MARKET, PLUGIN, install_plugin, plugin_market, write_json
variant = sys.argv[1] if len(sys.argv) > 1 else "enable-all-user"
name = "r8-untrusted-mcp-" + variant + ("-" + sys.argv[2] if len(sys.argv) > 2 else "")
FORMS, SOURCES = ("enable-all", "enabled-named", "plugin-repo-local", "plugin-repo-project", "plugin-repo", "plugin"), {"user-local": "user,local", "user": "user"}
form = next((f for f in FORMS if variant.startswith(f + "-")), None)
sources = SOURCES.get(variant[len(form) + 1:]) if form else None
if sources is None:
    raise SystemExit("unknown variant " + variant)
REPO_FILES = {"plugin-repo": ("settings.json", "settings.local.json"), "plugin-repo-local": ("settings.local.json",),
              "plugin-repo-project": ("settings.json",)}
cwd = make_repo(name)
ping_server(cwd)
marker, plugin_marker = os.path.join(cwd, "started.txt"), os.path.join(cwd, "started-plugin.txt")
for p in (marker, plugin_marker, os.path.join(cwd, ".claude", "settings.json"), os.path.join(cwd, ".claude", "settings.local.json")):
    if os.path.exists(p):
        os.remove(p)
write_json(os.path.join(cwd, ".mcp.json"), {"mcpServers": {"ping-project": stdio_server("started.txt")}})
h = home(name)
setup = {"setting_sources": sources}
user_file = os.path.join(h, ".claude", "settings.json")
if form == "enable-all":
    write_json(user_file, {"enableAllProjectMcpServers": True})
elif form == "enabled-named":
    write_json(user_file, {"enabledMcpjsonServers": ["ping-project"]})
else:
    setup["plugin_commands"] = install_plugin(plugin_market(name, plugin_marker), cwd, h, enabled=(form == "plugin"))
    if form in REPO_FILES:
        for f in REPO_FILES[form]:
            write_json(os.path.join(cwd, ".claude", f), {"enabledPlugins": {"%s@%s" % (PLUGIN, MARKET): True}})
        setup["repo_files"] = {f: read_json(os.path.join(cwd, ".claude", f)) for f in REPO_FILES[form]}
setup["user_settings_file"] = {k: v for k, v in (read_json(user_file) or {}).items() if k != "extraKnownMarketplaces"}
setup["servers_started_by_the_setup"] = dict(project=len(lines_of(marker)), plugin=len(lines_of(plugin_marker)))
for p in (marker, plugin_marker):  # count from the session's start on
    if os.path.exists(p):
        os.remove(p)
seen = []


def handler(req):
    seen.append(req)
    if req.get("tool_name", "").startswith("mcp__") and req["tool_name"].endswith("__ping"):
        return {"behavior": "allow", "updatedInput": req.get("input", {})}
    return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}


s = Session(name, mode="manual", extra=["--tools", "Bash"], hard_timeout=90, on_can_use_tool=handler, strict_mcp=False,
            setting_sources=sources, env={"HOME": h})
time.sleep(3.0)
s.note(setup=setup, before_first_message=True, servers_started=dict(project=len(lines_of(marker)), plugin=len(lines_of(plugin_marker))))
print("turn:", next_turn(s, "Reply with the single word: pong", 30))
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
servers = init.get("mcp_servers", [])
s.note(server_starts=dict(project=len(lines_of(marker)), plugin=len(lines_of(plugin_marker))), init_mcp_servers=servers,
       init_mcp_tools=[t for t in init.get("tools", []) if t.startswith("mcp__")],
       init_plugins=[p.get("name") if isinstance(p, dict) else p for p in init.get("plugins", [])],
       can_use_tool_requests=len(seen), result=(s.wait_for(lambda e: e.get("type") == "result", 1) or {}).get("result"))
print("starts:", len(lines_of(marker)), len(lines_of(plugin_marker)), "init servers:", servers, "plugins:", init.get("plugins"))
print("exit:", s.close())
