"""A minimal MCP server on stdio (python3 standard library only): answers initialize, tools/list (one tool, `ping`)
and tools/call. It writes one line to the file named in argv[1] when it starts, so the host can see that it ran."""
import json, os, sys, time

if len(sys.argv) > 1:
    with open(sys.argv[1], "a") as f:
        f.write("started pid=%d at %s\n" % (os.getpid(), time.strftime("%H:%M:%S")))
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    rid, method = m.get("id"), m.get("method")
    if rid is None:
        continue  # a notification
    if method == "initialize":
        res = {"protocolVersion": m.get("params", {}).get("protocolVersion", "2024-11-05"), "capabilities": {"tools": {}},
               "serverInfo": {"name": "ping-project", "version": "0.0.1"}}
    elif method == "tools/list":
        res = {"tools": [{"name": "ping", "description": "Answers pong.", "inputSchema": {"type": "object", "properties": {}}}]}
    elif method == "tools/call":
        res = {"content": [{"type": "text", "text": "pong"}]}
    else:
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}) + "\n")
        sys.stdout.flush()
        continue
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": rid, "result": res}) + "\n")
    sys.stdout.flush()
