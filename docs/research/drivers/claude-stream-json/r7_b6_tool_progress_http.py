"""B6: `tool_progress` for a tool of an HTTP MCP server on 127.0.0.1 (a tool that waits 70 s, or 45 s): is there a
heartbeat line every 30 s, as for a stdio server (part 3, section 3)?

The server `ping-http` (mcp_http_ping_server.py, in a thread of this script: Streamable HTTP, JSON answers) comes with
--mcp-config as type "http". One turn calls its tool once. A note lists every `tool_progress` line, every request the
server got (when, which JSON-RPC method, when it was answered), the tool's result and the times of the call
(time stamps of the lines).

argv[1]: how long the tool waits, in seconds (70: the question; 45: a control below the 60 s at which the first
         recordings of 70 s ended)
argv[2]: a label for the recording (1, 2): the same scenario, recorded more than once.
"""
import json, os, sys
from drv import Session, make_repo, OUT
from mcp_http_ping_server import serve
from r5_lib import allow_all, asked, results
from r6_lib import raw
from r7_lib import is_tool_result, is_tool_use, tool_results
secs = int(sys.argv[1]) if len(sys.argv) > 1 else 70
run = sys.argv[2] if len(sys.argv) > 2 else "1"
name = "r7-tool-progress-http-%d-%s" % (secs, run)
make_repo(name)
srv = serve(sleep=secs)
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-http": {"type": "http", "url": srv.url}}}, f)
seen = []
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=200, on_can_use_tool=allow_all(seen))
s.user("Call the tool mcp__ping-http__ping once (it is slow: wait for it) and reply with exactly what it returned. "
       "If the call fails, reply with the error text. Do not try again.")
print("result:", s.wait_results(1, 170))
rows = [r for r in raw(s) if r["dir"] == "recv" and isinstance(r["msg"], dict)]
use = [r["t"] for r in rows if is_tool_use("mcp__ping-http__ping")(r["msg"])]
res = [r["t"] for r in rows if is_tool_result()(r["msg"])]
progress = [dict({k: v for k, v in r["msg"].items() if k not in ("uuid", "session_id")}, _t=r["t"]) for r in rows if r["msg"].get("type") == "tool_progress"]
s.note(tool_progress_lines=progress, server_requests=srv.hits, tool_results=tool_results(s), can_use_tool_requests=asked(seen),
       seconds_from_tool_use_to_tool_result=round(res[0] - use[0], 3) if use and res else None,
       seconds_from_tool_use_to_each_tool_progress=[round(p["_t"] - use[0], 3) for p in progress] if use else None)
print("tool_progress:", [p.get("elapsed_time_seconds") for p in progress], "server:", srv.hits)
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
srv.stop()
