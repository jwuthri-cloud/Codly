"""E1: a plugin named by a file of the folder, and a plugin's HOOKS, in a folder you have not trusted. No run passes
--strict-mcp-config (dropping it is what the waiting code does there); --setting-sources is `user` or `user,local`.

A THROW-AWAY HOME at every run (no login: each message ends at once with "Not logged in"; the recordings show what
is LOADED and STARTED, and what a hook does). Nothing of the user's own Claude files is read or written.

The plugin `ping-plugin` (r9_lib.plugin_market, a local marketplace folder `codly-test-market` under the capture
folder) brings the stdio server `ping-plugged` (one line in started-plugin.txt per start) and two hooks, SessionStart
and UserPromptSubmit, each appends its event's name to hook-ran.txt in the work folder.

argv[1]: <form>-<src>, <src> is `user` or `user-local` (--setting-sources user / user,local)
  forms where the user NEVER installed the plugin (no `claude plugin` command ran):
         repo-both        the work folder's `.claude/settings.json` AND `.claude/settings.local.json` both declare the
                          marketplace (`extraKnownMarketplaces`, a `directory` source: the market folder) and turn the
                          plugin on (`enabledPlugins`)
         repo-local       the same in `.claude/settings.local.json` only
         user-declared    control: the same two keys in the throw-away HOME's USER settings file, no repo file
  forms where the user installed it (`claude plugin marketplace add`, `claude plugin install --scope user`):
         on               control: installed and on for the user, no repo file
         off              control: then `claude plugin disable --scope user`, no repo file
         off-repo-local   turned off for the user; the work folder's `.claude/settings.local.json` says
                          `"enabledPlugins": {"ping-plugin@codly-test-market": true}` (part 6's form, now with hooks)
argv[2]: a label (1, 2: the same scenario again), added to the fixture's name

Each run starts the CLI twice in the same HOME and folder (fixtures `-a` and `-b`): a CLI may fetch a declared
marketplace in the background and load its plugin only at a later start. Each process gets one message. The driver
allows ONE tool (the plugin's ping) and denies every other request (none comes without a login). No mcp_status is
sent: init says what was loaded.
"""
import os, sys, time
from drv import Session, make_repo
from r5_lib import lines_of, next_turn
from r7_lib import home, read_json
from r8_lib import MARKET, PLUGIN, install_plugin, write_json
from r9_lib import PLUGIN_ID, home_files, plugin_market
variant = sys.argv[1] if len(sys.argv) > 1 else "repo-both-user-local"
label = sys.argv[2] if len(sys.argv) > 2 else "1"
name = "r9-plugin-from-repo-%s-%s" % (variant, label)
FORMS = ("repo-both", "repo-local", "user-declared", "off-repo-local", "on", "off")
SOURCES = {"user-local": "user,local", "user": "user"}
form = next((f for f in FORMS if variant.startswith(f + "-") and variant[len(f) + 1:] in SOURCES), None)
if form is None:
    raise SystemExit("unknown variant " + variant)
sources = SOURCES[variant[len(form) + 1:]]
cwd = make_repo(name)
server_marker, hook_marker = os.path.join(cwd, "started-plugin.txt"), os.path.join(cwd, "hook-ran.txt")
repo_files = {f: os.path.join(cwd, ".claude", f) for f in ("settings.json", "settings.local.json")}
for p in [server_marker, hook_marker] + list(repo_files.values()):
    if os.path.exists(p):
        os.remove(p)
h = home(name)
market, _ = plugin_market(name, server_marker, hook_marker)
DECLARE = {"extraKnownMarketplaces": {MARKET: {"source": {"source": "directory", "path": market}}}, "enabledPlugins": {PLUGIN_ID: True}}
setup = {"setting_sources": sources}
user_file = os.path.join(h, ".claude", "settings.json")
if form in ("on", "off", "off-repo-local"):
    setup["plugin_commands"] = install_plugin(market, cwd, h, enabled=(form == "on"))
if form == "repo-both":
    for p in repo_files.values():
        write_json(p, DECLARE)
elif form == "repo-local":
    write_json(repo_files["settings.local.json"], DECLARE)
elif form == "user-declared":
    write_json(user_file, DECLARE)
elif form == "off-repo-local":
    write_json(repo_files["settings.local.json"], {"enabledPlugins": {PLUGIN_ID: True}})
setup["repo_files"] = {f: read_json(p) for f, p in repo_files.items() if os.path.exists(p)}
setup["user_settings_file"] = read_json(user_file)
setup["plugin_files_in_the_home"], _ = home_files(h)
setup["by_the_setup"] = dict(server_starts=len(lines_of(server_marker)), hook_lines=lines_of(hook_marker))
for p in (server_marker, hook_marker):  # count from the first session's start on
    if os.path.exists(p):
        os.remove(p)
seen = []


def handler(req):
    seen.append(req)
    if req.get("tool_name", "").startswith("mcp__") and req["tool_name"].endswith("__ping"):
        return {"behavior": "allow", "updatedInput": req.get("input", {})}
    return {"behavior": "deny", "message": "The user denied this action.", "interrupt": False}


for part in ("a", "b"):
    s = Session("%s-%s" % (name, part), repo=name, mode="manual", extra=["--tools", "Bash"], hard_timeout=90, on_can_use_tool=handler,
                strict_mcp=False, setting_sources=sources, env={"HOME": h})
    time.sleep(4.0)
    if part == "a":
        s.note(setup=setup)
    s.note(before_first_message=True, server_starts=len(lines_of(server_marker)), hook_lines=lines_of(hook_marker))
    print(part, "turn:", next_turn(s, "Reply with the single word: pong", 30))
    time.sleep(1.0)
    init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5) or {}
    mine, others = home_files(h)
    s.note(after_the_turn=True, server_starts=len(lines_of(server_marker)), hook_lines=lines_of(hook_marker),
           init_mcp_servers=init.get("mcp_servers", []), init_mcp_tools=[t for t in init.get("tools", []) if t.startswith("mcp__")],
           init_names_the_plugin=any(isinstance(p, dict) and p.get("name") == PLUGIN for p in init.get("plugins", [])),
           plugin_files_in_the_home=mine, can_use_tool_requests=len(seen),
           result=(s.wait_for(lambda e: e.get("type") == "result", 1) or {}).get("result"))
    print(part, "server starts:", len(lines_of(server_marker)), "hooks:", lines_of(hook_marker), "init servers:", init.get("mcp_servers"),
          "plugin in init:", any(isinstance(p, dict) and p.get("name") == PLUGIN for p in init.get("plugins", [])), "home files:", mine, "others:", others)
    print(part, "exit:", s.close())
