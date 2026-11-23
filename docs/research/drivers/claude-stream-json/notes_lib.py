"""Helpers for the notes builders: every JSON line they quote is copied from a sanitized fixture.

Shrinking rule for quoted lines (the fixture always has the full line):
  - an array with more than 3 items shows its first 2 items and "…(N more)"
  - a string longer than 320 characters is cut and ends with "…"
  - the keys in HEAVY are replaced by "…" unless a section asks for them
  - slim(): assistant, user and result lines keep only the keys named there; "…":"…" stands for the keys left out
    (brief=True also leaves out tool_use_result and tool_result_meta of user lines)
"""
import json
import os

OUT = os.path.abspath(os.environ.get("CAPTURE_DIR") or os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIXTURES = os.environ.get("FIXTURES_DIR") or os.path.join(OUT, "fixtures")
HEAVY = ("usage", "modelUsage", "model_usage_since_last_result", "subagent_stats")
RESULT_KEYS = ("type", "subtype", "is_error", "num_turns", "result", "stop_reason", "terminal_reason", "errors", "local_command", "origin",
               "queued_turn_count", "result_index", "user_message_uuid", "user_message_uuids")


def fx(name):
    return [json.loads(l) for l in open(os.path.join(FIXTURES, name + ".ndjson"))]


def shrink(v, keep=(), maxstr=320, maxlist=3, key=None):
    if isinstance(v, dict):
        return {k: ("…" if (k in HEAVY and k not in keep) else shrink(x, keep, maxstr, maxlist, k)) for k, x in v.items()}
    if isinstance(v, list):
        if len(v) > maxlist and key not in keep:
            return [shrink(x, keep, maxstr, maxlist) for x in v[:2]] + ["…(%d more)" % (len(v) - 2)]
        return [shrink(x, keep, maxstr, maxlist) for x in v]
    if isinstance(v, str) and len(v) > maxstr:
        return v[:maxstr] + "…"
    return v


def j(o, **kw):
    return json.dumps(shrink(o, **kw), ensure_ascii=False, separators=(",", ":"))


def block(lines, lang="json"):
    return "```%s\n" % lang + "\n".join(lines) + "\n```\n"


def is_cli(l):
    return not any(k.startswith("_") for k in l)


def quiet(l):
    """Not a stream event, a token counter, a command list or a rate-limit line."""
    return (l.get("type") not in ("stream_event", "rate_limit_event") and l.get("subtype") not in ("thinking_tokens", "commands_changed")
            and "_note" not in l and set(l) != {"_t"})


def slim(l, text=200, brief=False):
    """A short form of the long line kinds. Every other line is returned as it is."""
    t = l.get("type")
    if t == "assistant":
        content = []
        for b in l["message"]["content"]:
            if b["type"] == "thinking":
                content.append({"type": "thinking", "…": "…"})
            elif b["type"] == "text":
                content.append({"type": "text", "text": b["text"] if len(b["text"]) <= text else b["text"][:text] + "…"})
            elif b["type"] == "tool_use":
                content.append({"type": "tool_use", "id": b["id"], "name": b["name"], "input": b["input"]})
            else:
                content.append(b)
        return {"type": "assistant", "message": {"model": l["message"].get("model"), "content": content, "…": "…"}, "…": "…"}
    if t == "user" and is_cli(l):
        out = {"type": "user", "message": l["message"]}
        for k in ("isReplay", "isSynthetic") if brief else ("tool_use_result", "tool_result_meta", "isReplay", "isSynthetic"):
            if k in l:
                out[k] = l[k]
        out["…"] = "…"
        return out
    if t == "result":
        out = {k: l[k] for k in RESULT_KEYS if k in l} | {"…": "…"}
        if isinstance(out.get("result"), str) and len(out["result"]) > text:
            out["result"] = out["result"][:text] + "…"
        return out
    if t == "system" and l.get("subtype") == "init":
        return {k: l[k] for k in ("type", "subtype", "model", "permissionMode", "tools", "mcp_servers") if k in l} | {"…": "…"}
    if t in ("system", "command_lifecycle", "tool_progress"):
        return {k: v for k, v in l.items() if k not in ("uuid", "session_id")}
    return l


def timeline(name, pred=quiet, start=None, stop=None, short=True, text=200, brief=False, **kw):
    """Lines with their time. `# t=…` lines are not JSON: seconds since the process started.
    start / stop: predicates that open and close the window (the stop line is included)."""
    out, t, on = [], 0.0, start is None
    for l in fx(name):
        if set(l) == {"_t"}:
            t = l["_t"]
            continue
        if "_t" in l:
            t = l["_t"]
        if not on and start(l):
            on = True
        if on and pred(l):
            out.append("# t=%.2f" % t)
            out.append(j(slim(l, text, brief) if short else l, **kw))
        if on and stop is not None and stop(l):
            break
    return out


def sent(name, kind=None):
    return [l["_sent"] for l in fx(name) if "_sent" in l and (kind is None or l["_sent"].get("type") == kind)]


def control(name, subtype=None, n=None, **kw):
    """Host control requests of a fixture with their answers, in order: [request line, response line, ...]."""
    lines = fx(name)
    out = []
    for i, l in enumerate(lines):
        s = l.get("_sent")
        if not s or s.get("type") != "control_request" or (subtype and s["request"]["subtype"] != subtype):
            continue
        out.append(j(s, **kw))
        for m in lines[i + 1:]:
            if m.get("type") == "control_response" and m["response"].get("request_id") == s["request_id"]:
                out.append(j(m, **kw))
                break
    return out if n is None else out[:2 * n]


def first(name, pred):
    for l in fx(name):
        if pred(l):
            return l
    return None


def argv_flags(name):
    """The scenario's own flags: what follows --setting-sources <x> in the recorded argv."""
    a = fx(name)[0]["_argv"]
    base = {"-p", "--input-format", "--output-format", "--verbose", "--include-partial-messages", "--permission-prompt-tool", "--session-id", "--model"}
    out, i = [], 1
    while i < len(a):
        if a[i] in ("-p", "--verbose", "--include-partial-messages"):
            i += 1
        elif a[i] in base:
            i += 2
        else:
            out.append(a[i] if not (a[i].startswith("{") or " " in a[i] or a[i] == "") else "'%s'" % a[i])
            i += 1
    return " ".join(out)
