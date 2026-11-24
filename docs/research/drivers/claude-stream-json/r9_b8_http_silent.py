"""B8: a slow tool of a streaming HTTP MCP server that sends nothing for more than 5 minutes. Part 6 (section 10) read
in the CLI's program file an idle limit of 300 s for servers that are not stdio; this records one call.

The server `ping-http` (mcp_http_ping_server.py, in a thread of this script, answer kind `sse`) comes with
--mcp-config as type "http": tools/call gets the event stream's headers at once, then NOTHING until the tool's result
at 330 s. One turn calls the tool once. A note: every `tool_progress` line, every request the server got (when, which
JSON-RPC method, when it was answered or that the client was gone), the tool's result, and the times of the call
(time stamps of the lines).

argv[1]: a label for the recording (1: one run is enough for this question).
"""
import json, os, sys
from drv import Session, make_repo, OUT
from mcp_http_ping_server import serve
from r5_lib import allow_all, asked, results
from r6_lib import raw
from r7_lib import is_tool_result, is_tool_use, tool_results
LABEL = sys.argv[1] if len(sys.argv) > 1 else "1"
SECS = 330
name = "r9-http-silent-" + LABEL
make_repo(name)
srv = serve(sleep=SECS, answer="sse")
cfg = os.path.join(OUT, "work", name + ".json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"ping-http": {"type": "http", "url": srv.url}}}, f)
seen = []
s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=SECS + 150, on_can_use_tool=allow_all(seen))
s.note(server_entry={"type": "http", "url": "http://127.0.0.1:<port>/mcp"}, tool_seconds=SECS)
s.user("Call the tool mcp__ping-http__ping once (it is very slow: wait for it) and reply with exactly what it returned. "
       "If the call fails, reply with the error text. Do not try again.")
print("result:", s.wait_results(1, SECS + 120))
rows = [r for r in raw(s) if r["dir"] == "recv" and isinstance(r["msg"], dict)]
use = [r["t"] for r in rows if is_tool_use("mcp__ping-http__ping")(r["msg"])]
res = [r["t"] for r in rows if is_tool_result()(r["msg"])]
progress = [dict({k: v for k, v in r["msg"].items() if k not in ("uuid", "session_id")}, _t=r["t"]) for r in rows if r["msg"].get("type") == "tool_progress"]
s.note(tool_progress_count=len(progress), tool_progress_first_and_last=progress[:1] + progress[-1:], server_requests=srv.hits,
       tool_results=tool_results(s), can_use_tool_requests=asked(seen),
       seconds_from_tool_use_to_tool_result=round(res[0] - use[0], 3) if use and res else None,
       seconds_from_tool_use_to_each_tool_progress=[round(p["_t"] - use[0], 3) for p in progress] if use else None)
print("tool_progress:", len(progress), "server:", srv.hits)
print("tool results:", tool_results(s))
print("results:", results(s), "cost:", s.cost(), "exit:", s.close())
srv.stop()
