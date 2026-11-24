"""Helpers shared by the r8_* scripts (part 6): the scenario's plugin and its local marketplace, the uuids and words of
a turn, counts from a session's transcript. Processes are found with r5_lib.Procs (the whole command text:
`Procs("sleep 211")`)."""
import glob
import json
import os
import shutil

from drv import OUT
from r5_lib import HERE
from r7_lib import claude_cli

MARKET, PLUGIN, PLUGIN_SERVER = "codly-test-market", "ping-plugin", "ping-plugged"


def write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(obj, f, indent=2)


def plugin_market(name, marker):
    """A local marketplace folder under the capture folder with ONE plugin, `ping-plugin`, whose `.mcp.json` brings the
    stdio server `ping-plugged` (mcp_ping_server.py; one line in `marker`, an absolute path, per start). Made new at
    each run (it is this script's own folder). Returns the folder."""
    d = os.path.join(OUT, "work", name + "-market")
    shutil.rmtree(d, ignore_errors=True)
    root = os.path.join(d, PLUGIN)
    write_json(os.path.join(d, ".claude-plugin", "marketplace.json"), {
        "name": MARKET, "owner": {"name": "demo"},
        "plugins": [{"name": PLUGIN, "source": "./" + PLUGIN, "description": "A test plugin with one MCP server."}]})
    write_json(os.path.join(root, ".claude-plugin", "plugin.json"), {"name": PLUGIN, "version": "0.0.1", "description": "A test plugin with one MCP server."})
    shutil.copy(os.path.join(HERE, "mcp_ping_server.py"), os.path.join(root, "mcp_ping_server.py"))
    write_json(os.path.join(root, ".mcp.json"), {"mcpServers": {PLUGIN_SERVER: {
        "command": "python3", "args": ["${CLAUDE_PLUGIN_ROOT}/mcp_ping_server.py", marker, "label=plugin"]}}})
    return d


def install_plugin(market_dir, cwd, home_dir, enabled):
    """`claude plugin marketplace add` and `claude plugin install --scope user` with the throw-away HOME (they write
    only under it); `claude plugin disable --scope user` after it when enabled is False. Returns what each command
    gave (exit code, first line of its output)."""
    steps = [["plugin", "marketplace", "add", market_dir], ["plugin", "install", "%s@%s" % (PLUGIN, MARKET), "--scope", "user"]]
    if not enabled:
        steps.append(["plugin", "disable", "%s@%s" % (PLUGIN, MARKET), "--scope", "user"])
    out = []
    for args in steps:
        r = claude_cli(args, cwd, {"HOME": home_dir})
        text = (r.stdout.strip() or r.stderr.strip()).splitlines()
        out.append({"command": "claude " + " ".join(a if a != market_dir else "<market folder>" for a in args), "exit_code": r.returncode,
                    "first_line_of_its_output": text[0] if text else ""})
    return out


def assistant_uuids(s, start=0):
    """The `uuid` of every complete assistant line of the main thread from event index `start` on."""
    with s.cv:
        return [e.get("uuid") for e in s.events[start:] if e.get("type") == "assistant" and not e.get("parent_tool_use_id")]


def words_in(text, words):
    """Which of these words the text names (lower case)."""
    t = (text or "").lower()
    return [w for w in words if w in t]


def last_result(s, start=0):
    with s.cv:
        r = [e for e in s.events[start:] if e.get("type") == "result"]
    return r[-1] if r else {}


def transcript(sid, words=()):
    """What the session's own transcript (~/.claude/projects/<folder>/<sid>.jsonl, written by the CLI) holds: whether it
    exists, its number of lines, how many lines name each of these words, and how many session files its folder has.
    Counts only: the transcript itself has machine data and is never copied."""
    paths = glob.glob(os.path.join(os.path.expanduser("~"), ".claude", "projects", "*", sid + ".jsonl"))
    if len(paths) != 1:
        return {"transcript_found": False}
    lines = open(paths[0]).read().splitlines()
    return {"transcript_found": True, "lines": len(lines), "lines_that_name": {w: sum(1 for l in lines if w in l.lower()) for w in words},
            "session_files_in_its_folder": len(glob.glob(os.path.join(os.path.dirname(paths[0]), "*.jsonl")))}
