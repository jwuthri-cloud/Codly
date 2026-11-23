"""A minimal MCP server over Streamable HTTP on 127.0.0.1 (python3 standard library only), run in a thread of the
driver: POST /mcp answers initialize, tools/list (one tool, `ping`) and tools/call with a JSON body; a notification
gets 202 with no body; GET and DELETE get 405 (the server offers no event stream and no session end).

    srv = serve(sleep=70)        # the tool waits that long before it answers
    srv.url, srv.hits            # the URL to configure; one entry per request the server got (no header values)
    srv.stop()
"""
import http.server
import json
import threading
import time


class _Handler(http.server.BaseHTTPRequestHandler):
    def _send(self, code, body=None):
        data = json.dumps(body).encode() if body is not None else b""
        self.send_response(code)
        if body is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        try:
            m = json.loads(self.rfile.read(n) or b"{}")
        except ValueError:
            m = {}
        srv = self.server
        hit = {"at": round(time.time() - srv.t0, 3), "http": "POST", "jsonrpc_method": m.get("method")}
        srv.hits.append(hit)
        rid, method = m.get("id"), m.get("method")
        if rid is None:
            self._send(202)
            return
        if method == "initialize":
            res = {"protocolVersion": m.get("params", {}).get("protocolVersion", "2025-03-26"), "capabilities": {"tools": {}},
                   "serverInfo": {"name": "ping-http", "version": "0.0.1"}}
        elif method == "tools/list":
            res = {"tools": [{"name": "ping", "description": "Answers pong.", "inputSchema": {"type": "object", "properties": {}}}]}
        elif method == "tools/call":
            time.sleep(srv.sleep)
            res = {"content": [{"type": "text", "text": "pong after %g s" % srv.sleep}]}
        else:
            self._send(200, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}})
            return
        try:
            self._send(200, {"jsonrpc": "2.0", "id": rid, "result": res})
            hit["answered_at"] = round(time.time() - srv.t0, 3)
        except OSError as e:  # the client went away before the answer
            hit["answer_failed"] = type(e).__name__

    def _no(self):
        self.server.hits.append({"at": round(time.time() - self.server.t0, 3), "http": self.command})
        self._send(405)

    do_GET = do_DELETE = _no

    def log_message(self, *a):
        pass


def serve(sleep=0.0):
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    srv.daemon_threads = True
    srv.sleep, srv.hits, srv.t0 = sleep, [], time.time()
    srv.url = "http://127.0.0.1:%d/mcp" % srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    def stop():
        srv.shutdown()
        srv.server_close()
    srv.stop = stop
    return srv
