"""A minimal MCP server over Streamable HTTP on 127.0.0.1 (python3 standard library only), run in a thread of the
driver: POST /mcp answers initialize, tools/list (one tool, `ping`) and tools/call; a notification gets 202 with no
body; GET and DELETE get 405 (the server offers no stream of its own and no session end).

    srv = serve(sleep=70)                  # the tool waits that long before it answers
    srv = serve(sleep=70, answer="sse")    # tools/call is answered with an event stream: its headers at once, the
                                           # result as one event when the tool is done
    srv = serve(sleep=70, answer="progress", every=10)   # the same stream, with a `notifications/progress` event every
                                           # `every` seconds while the tool works (only when the request asked for
                                           # progress: `params._meta.progressToken`)
    srv.url, srv.hits            # the URL to configure; one entry per request the server got (no header values)
    srv.stop()

Each entry of `hits` gets `answered_at` once the server has written its whole answer (also a 202 or a 405), or
`answer_failed` when the client was gone: an entry with neither is a request the server is still working on.
"""
import http.server
import json
import threading
import time


class _Handler(http.server.BaseHTTPRequestHandler):
    def _send(self, hit, code, body=None):
        data = json.dumps(body).encode() if body is not None else b""
        try:
            self.send_response(code)
            if body is not None:
                self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            self._answered(hit)
        except OSError as e:  # the client went away before the answer
            hit["answer_failed"] = type(e).__name__

    def _answered(self, hit):
        hit["answered_at"] = round(time.time() - self.server.t0, 3)

    def _event(self, obj):
        self.wfile.write(b"event: message\ndata: " + json.dumps(obj).encode() + b"\n\n")
        self.wfile.flush()

    def _stream_call(self, hit, rid, token):
        """tools/call as an event stream: headers now, progress events while the tool works, then the result."""
        srv = self.server
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            self.wfile.flush()
            hit["headers_sent_at"] = round(time.time() - srv.t0, 3)
            start, sent = time.time(), 0
            while time.time() - start < srv.sleep:
                time.sleep(min(srv.every if srv.answer == "progress" else srv.sleep, srv.sleep - (time.time() - start)))
                if srv.answer == "progress" and token is not None and time.time() - start < srv.sleep:
                    sent += 1
                    self._event({"jsonrpc": "2.0", "method": "notifications/progress",
                                 "params": {"progressToken": token, "progress": sent, "message": "working"}})
                    hit["progress_sent"] = sent
            self._event({"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": "pong after %g s" % srv.sleep}]}})
            self._answered(hit)
        except OSError as e:
            hit["answer_failed"] = type(e).__name__

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
            self._send(hit, 202)
            return
        if method == "initialize":
            res = {"protocolVersion": m.get("params", {}).get("protocolVersion", "2025-03-26"), "capabilities": {"tools": {}},
                   "serverInfo": {"name": "ping-http", "version": "0.0.1"}}
        elif method == "tools/list":
            res = {"tools": [{"name": "ping", "description": "Answers pong.", "inputSchema": {"type": "object", "properties": {}}}]}
        elif method == "tools/call":
            token = (m.get("params", {}).get("_meta") or {}).get("progressToken")
            hit["progress_token_in_request"] = token is not None
            if srv.answer in ("sse", "progress"):
                self._stream_call(hit, rid, token)
                return
            time.sleep(srv.sleep)
            res = {"content": [{"type": "text", "text": "pong after %g s" % srv.sleep}]}
        else:
            self._send(hit, 200, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}})
            return
        self._send(hit, 200, {"jsonrpc": "2.0", "id": rid, "result": res})

    def _no(self):
        hit = {"at": round(time.time() - self.server.t0, 3), "http": self.command}
        self.server.hits.append(hit)
        self._send(hit, 405)

    do_GET = do_DELETE = _no

    def log_message(self, *a):
        pass


def serve(sleep=0.0, answer="json", every=10.0):
    if answer not in ("json", "sse", "progress"):
        raise ValueError("answer must be json, sse or progress")
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    srv.daemon_threads = True
    srv.sleep, srv.answer, srv.every, srv.hits, srv.t0 = sleep, answer, every, [], time.time()
    srv.url = "http://127.0.0.1:%d/mcp" % srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    def stop():
        srv.shutdown()
        srv.server_close()
    srv.stop = stop
    return srv
