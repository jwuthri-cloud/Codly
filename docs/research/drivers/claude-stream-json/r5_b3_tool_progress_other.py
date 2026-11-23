"""B3: `tool_progress` lines for a tool that is not a top-level Bash call, past 90 s.

argv[1]: mcp       a slow MCP tool (mcp_ping_server.py with sleep=100), from --mcp-config
         subagent  a foreground subagent whose Bash call runs 100 s
         webfetch  WebFetch of a local page that answers after 40 s (only if the CLI offers the tool)
"""
import http.server, json, os, sys, threading, time
from drv import Session, make_repo, OUT
from r5_lib import allow_all, asked, ping_server, results, stdio_server
variant = sys.argv[1] if len(sys.argv) > 1 else "mcp"
name = "r5-tool-progress-" + variant
cwd = make_repo(name)
seen = []

def progress(s):
    with s.cv:
        return [{k: v for k, v in e.items() if k not in ("uuid", "session_id")} for e in s.events if e.get("type") == "tool_progress"]

if variant == "mcp":
    ping_server(cwd)
    cfg = os.path.join(OUT, "work", name + ".json")
    with open(cfg, "w") as f:
        json.dump({"mcpServers": {"ping-slow": stdio_server("started.txt", sleep=100)}}, f)
    s = Session(name, extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=230, on_can_use_tool=allow_all(seen))
    s.user("Call the tool mcp__ping-slow__ping once (it is slow: wait for it) and reply with what it returned.")
    print("result:", s.wait_results(1, 200))
elif variant == "subagent":
    s = Session(name, extra=["--tools", "Bash,Agent"], hard_timeout=260, on_can_use_tool=allow_all(seen))
    s.user("Use the Agent tool with run_in_background set to false to have a subagent run the Bash command "
           "`echo begin; sleep 100; echo end` (one foreground call, with a timeout of 150000 ms) and report its output. "
           "Then tell me what it reported.")
    print("result:", s.wait_results(1, 230))
else:
    class Slow(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            time.sleep(40)
            body = b"<html><body><p>The word is: pineapple</p></body></html>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Slow)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    s = Session(name, extra=["--tools", "Bash,WebFetch"], hard_timeout=150, on_can_use_tool=allow_all(seen))
    s.user("If you have a tool named WebFetch, use it once on http://127.0.0.1:%d/slow and reply with the word that the page "
           "names. If you have no such tool, reply with the single word: none" % port)
    print("result:", s.wait_results(1, 120))
    srv.shutdown()
    srv.server_close()
init = s.wait_for(lambda e: e.get("type") == "system" and e.get("subtype") == "init", 5)
p = progress(s)
s.note(init_tools=[t for t in (init or {}).get("tools", []) if not t.startswith("mcp__") or "ping-slow" in t],
       tool_progress_lines=len(p), can_use_tool_requests=asked(seen))
for x in p:
    print("tool_progress:", json.dumps(x))
print("tool_progress lines:", len(p), "results:", results(s), "cost:", s.cost())
print("exit:", s.close())
