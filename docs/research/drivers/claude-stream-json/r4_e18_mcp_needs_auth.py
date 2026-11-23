"""E18: an MCP server that needs a sign-in. A tiny local HTTP server on 127.0.0.1 answers every request with 401 and
a WWW-Authenticate header (404 for the /.well-known/ lookups); it is configured with --mcp-config as type "http".

Recorded: mcp_status before and after the first turn, system/init.mcp_servers, mcp_reconnect, mcp_toggle off and on.
Each request the fake server got is noted in the recording (method, path, names of the headers; no header values).
"""
import http.server, json, os, socket, threading, time
from drv import Session, OUT

PORT = int(os.environ.get("FAKE_MCP_PORT", "48731"))
hits = []


class Handler(http.server.BaseHTTPRequestHandler):
    def _answer(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        method = None
        try:
            method = json.loads(body).get("method")
        except Exception:
            pass
        well_known = self.path.startswith("/.well-known/")
        hits.append({"method": self.command, "path": self.path, "jsonrpc_method": method, "answer": 404 if well_known else 401,
                     "has_authorization_header": "authorization" in {k.lower() for k in self.headers.keys()}})
        if well_known:
            self.send_response(404)
            self.send_header("Content-Type", "application/json")
            data = b'{"error":"not_found"}'
        else:
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Bearer realm="needs-login", error="invalid_token", error_description="Sign in first"')
            self.send_header("Content-Type", "application/json")
            data = b'{"error":"unauthorized"}'
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = do_POST = do_DELETE = do_OPTIONS = do_HEAD = _answer

    def log_message(self, *a):
        pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
threading.Thread(target=srv.serve_forever, daemon=True).start()
cfg = os.path.join(OUT, "work", "r4-mcp-needs-auth.json")
with open(cfg, "w") as f:
    json.dump({"mcpServers": {"needs-login": {"type": "http", "url": "http://127.0.0.1:%d/mcp" % PORT}}}, f)

s = Session("r4-mcp-needs-auth", extra=["--tools", "Bash", "--mcp-config", cfg], hard_timeout=150)
def flush(label):
    global hits
    got, hits = hits, []
    s.note(fake_server_requests=got, until=label)

def ask(sub, wait=30, **f):
    r = s.control(sub, wait=wait, **f)
    resp = (r or {}).get("response", {})
    print("%-14s %-45s -> %s" % (sub, f, str({k: v for k, v in resp.items() if k != "request_id"})[:500]))
ask("mcp_status")
s.user("Reply with the single word: pong")
print("result:", s.wait_results(1, 60))
flush("first turn done")
ask("mcp_status")
ask("mcp_reconnect", serverName="needs-login")
flush("mcp_reconnect answered")
ask("mcp_status")
ask("mcp_toggle", serverName="needs-login", enabled=False)
ask("mcp_status")
ask("mcp_toggle", serverName="needs-login", enabled=True)
time.sleep(2.0)
flush("mcp_toggle on + 2 s")
ask("mcp_status")
s.user("Reply with the single word: ping")
print("result:", s.wait_results(2, 60))
init2 = [e for e in s.events if e.get("type") == "system" and e.get("subtype") == "init"][-1]
print("init 2 mcp_servers:", init2.get("mcp_servers"))
flush("second turn done")
print("cost:", s.cost(), "exit:", s.close())
srv.shutdown()
srv.server_close()
print("fake server stopped")
