"""Compact timeline of a raw recording: python3 tl.py <scenario> [maxlen]"""
import json, os, sys
OUT = os.path.abspath(os.environ.get("CAPTURE_DIR") or os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

def brief(m, n):
    if not isinstance(m, dict):
        return str(m)[:n]
    t = m.get("type")
    if t == "assistant":
        parts = []
        for b in m["message"]["content"]:
            if b["type"] == "text": parts.append("text=%r" % b["text"][:200])
            elif b["type"] == "tool_use": parts.append("tool_use %s %s" % (b["name"], json.dumps(b["input"])[:200]))
            else: parts.append(b["type"])
        return "assistant[%s] parent=%s %s" % (m["message"].get("model"), m.get("parent_tool_use_id"), "; ".join(parts))
    if t == "user" and isinstance(m.get("message", {}).get("content"), list) and "session_id" in m and m.get("uuid") and "tool_use_result" in m or (t == "user" and m.get("timestamp")):
        c = m["message"]["content"]
        if isinstance(c, list):
            parts = []
            for b in c:
                if b.get("type") == "tool_result": parts.append("tool_result err=%s %r" % (b.get("is_error"), json.dumps(b.get("content"))[:300]))
                else: parts.append(json.dumps(b)[:200])
            c = "; ".join(parts)
        extra = {k: m[k] for k in m if k not in ("type", "message", "session_id", "uuid", "timestamp", "tool_use_result")}
        return "user(recv) %s %s" % (extra, str(c)[:n])
    if t == "result":
        keys = ("subtype", "is_error", "num_turns", "result", "terminal_reason", "stop_reason", "total_cost_usd", "queued_turn_count", "result_index", "errors", "permission_denials", "api_error_status")
        return "RESULT " + json.dumps({k: m[k] for k in keys if k in m})[:n]
    if t == "system" and m.get("subtype") == "init":
        return "system/init model=%s mode=%s tools=%d mcp=%s" % (m.get("model"), m.get("permissionMode"), len(m.get("tools", [])), json.dumps(m.get("mcp_servers")))
    if t == "system" and m.get("subtype") == "commands_changed":
        return "system/commands_changed (%d commands)" % len(m.get("commands", []))
    d = {k: v for k, v in m.items() if k not in ("session_id", "uuid")}
    return json.dumps(d)[:n]

def main(name, n=500, skip=("thinking_tokens",)):
    p = name if os.path.sep in name else os.path.join(OUT, "raw", name + ".ndjson")
    for line in open(p):
        r = json.loads(line)
        m = r["msg"]
        if r["dir"] == "recv" and isinstance(m, dict) and (m.get("type") == "stream_event" or m.get("subtype") in skip):
            continue
        print("%7.2f %-6s %s" % (r["t"], r["dir"], brief(m, n) if r["dir"] == "recv" else json.dumps(m)[:n]))

if __name__ == "__main__":
    main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 500)
