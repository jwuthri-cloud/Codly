"""B10: a tool that takes 70 s on an HTTP MCP server of the other kinds. Part 5 (section 6) recorded a server that sends
nothing until the tool is done (one JSON body): the call ended at 60 s with "The operation timed out.".

The server `ping-http` (mcp_http_ping_server.py, in a thread of this script) comes with --mcp-config as type "http".
One turn calls its tool once. A note lists every `tool_progress` line, every request the server got (when, which
JSON-RPC method, when it was answered, whether the call asked for progress), the tool's result and the times of the
call (time stamps of the lines).

argv[1]: sse           tools/call is answered with an event stream: headers at once, the result as one event at 70 s
         progress      the same, plus a `notifications/progress` event every 10 s (when the call has a progressToken)
         json-env      the JSON answer of part 5, with MCP_TOOL_TIMEOUT=120000 in the CLI's environment (a variable
                       the CLI's bundle reads for the limit of an HTTP request: bgrep.py MCP_TOOL_TIMEOUT)
         json-timeout  the JSON answer, with `"timeout": 120000` in the server's entry of the --mcp-config file (the
                       per-server key the bundle describes as "Per-server tool-call timeout in milliseconds")
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import json, os, sys
from drv import Session, make_repo, OUT
from mcp_http_ping_server import serve
from r5_lib import allow_all, asked, results
from r6_lib import raw
from r7_lib import is_tool_result, is_tool_use, tool_results
VARIANT = sys.argv[1] if len(sys.argv) > 1 else "sse"
LABEL = sys.argv[2] if len(sys.argv) > 2 else "1"
if VARIANT not in ("sse", "progress", "json-env", "json-timeout"):
    raise SystemExit("unknown variant " + VARIANT)
SECS = 70
name = "r8-http-slow-%s-%s" % (VARIANT, LABEL)
make_repo(name)
srv = serve(sleep=SECS, answer=VARIANT if VARIANT in ("sse", "progress") else "json")
entry = {"type": "http", "url": srv.url} | ({"timeout": 120000} if VARIANT == "json-timeout" else {})
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-http": entry}}, f)
seen = []
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=220, on_can_use_tool=allow_all(seen),
            env={"MCP_TOOL_TIMEOUT": "120000"} if VARIANT == "json-env" else None)
s.note(server_entry={k: (v if k != "url" else "http://127.0.0.1:<port>/mcp") for k, v in entry.items()})
s.user("Call the tool mcp__ping-http__ping once (it is slow: wait for it) and reply with exactly what it returned. "
       "If the call fails, reply with the error text. Do not try again.")
print("result:", s.wait_results(1, 200))
rows = [r for r in raw(s) if r["dir"] == "recv" and isinstance(r["msg"], dict)]
use = [r["t"] for r in rows if is_tool_use("mcp__ping-http__ping")(r["msg"])]
res = [r["t"] for r in rows if is_tool_result()(r["msg"])]
progress = [dict({k: v for k, v in r["msg"].items() if k not in ("uuid", "session_id")}, _t=r["t"]) for r in rows if r["msg"].get("type") == "tool_progress"]
s.note(tool_progress_lines=progress, server_requests=srv.hits, tool_results=tool_results(s), can_use_tool_requests=asked(seen),
       seconds_from_tool_use_to_tool_result=round(res[0] - use[0], 3) if use and res else None,
       seconds_from_tool_use_to_each_tool_progress=[round(p["_t"] - use[0], 3) for p in progress] if use else None)
print("tool_progress:", [(p.get("elapsed_time_seconds"), p.get("heartbeat")) for p in progress], "server:", srv.hits)
print("tool results:", tool_results(s))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
srv.stop()
