"""A minimal MCP server on stdio (python3 standard library only): answers initialize, tools/list (one tool, `ping`)
and tools/call. It writes one line to the file named in argv[1] when it starts, so the host can see that it ran.

Options after argv[1], each as key=value (part 3 scenarios):
  label=<text>   the tool answers "pong from <text>" (tells two servers with the same name apart)
  sleep=<secs>   the tool waits that long before it answers (a slow tool)
  need=<file>    the server exits at once with code 1 while that file does not exist (a server that fails first)
"""
import json, os, sys, time

opts = dict(a.split("=", 1) for a in sys.argv[2:] if "=" in a)
if len(sys.argv) > 1:
    with open(sys.argv[1], "a") as f:
        f.write("started pid=%d at %s%s\n" % (os.getpid(), time.strftime("%H:%M:%S"), " label=" + opts["label"] if "label" in opts else ""))
if "need" in opts and not os.path.exists(opts["need"]):
    sys.stderr.write("mcp_ping_server: %s is missing, exiting\n" % os.path.basename(opts["need"]))
    sys.exit(1)
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
        if "sleep" in opts:
            time.sleep(float(opts["sleep"]))
        res = {"content": [{"type": "text", "text": "pong from " + opts["label"] if "label" in opts else "pong"}]}
    else:
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}) + "\n")
        sys.stdout.flush()
        continue
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": rid, "result": res}) + "\n")
    sys.stdout.flush()
