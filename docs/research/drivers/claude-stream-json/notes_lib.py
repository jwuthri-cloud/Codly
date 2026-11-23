"""Helpers for the notes builders: every JSON line they quote is copied from a sanitized fixture.

Shrinking rule for quoted lines (the fixture always has the full line):
  - an array with more than 3 items shows its first 2 items and "…(N more)"
  - a string longer than 320 characters is cut and ends with "…"
  - the keys in HEAVY are replaced by "…" unless a section asks for them
  - slim(): assistant, user and result lines keep only the keys named there; "…":"…" stands for the keys left out
    (brief=True also leaves out tool_use_result and tool_result_meta of user lines)

check(): the builders of part 3 and part 4 state numbers and words about a fixture only through it, so a build fails
when a recording no longer says what the text says.
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


# ---- shared by the builders of part 3 and part 4

def check(ok, what):
    if not ok:
        raise SystemExit("the text no longer matches the fixtures: " + what)


def flags(name):
    return "`%s`" % argv_flags(name)


def slimp(l, text=200, brief=False):
    """slim(), but a line of a subagent keeps the keys that say so."""
    out = slim(l, text, brief)
    if l.get("type") in ("assistant", "user") and is_cli(l) and l.get("parent_tool_use_id"):
        tail = out.pop("…")
        out["parent_tool_use_id"] = l["parent_tool_use_id"]
        if "agent_id" in l:
            out["agent_id"] = l["agent_id"]
        out["…"] = tail
    return out


def tl(name, pred, start=None, stop=None, text=200, brief=True, **kw):
    """Lines with their time (`# t=…`: seconds since the process started). start / stop: predicates that open and
    close the window (the stop line is included)."""
    out, t, on = [], 0.0, start is None
    for l in fx(name):
        if set(l) == {"_t"}:
            t = l["_t"]
            continue
        if "_t" in l:
            t = l["_t"]
        if not on and start(l):
            on = True
        if on and quiet(l) and pred(l):
            out.append("# t=%.2f" % t)
            out.append(j(slimp(l, text, brief), **kw))
        if on and stop is not None and stop(l):
            break
    return out


def kind(l):
    if "_sent" in l:
        return "sent"
    if "_meta" in l:
        return "meta"
    if "_stderr" in l:
        return "stderr"
    return l.get("type")


def sub(l):
    return l.get("subtype") if l.get("type") == "system" else None


TASK = ("task_started", "task_updated", "task_notification", "background_tasks_changed")


def is_tool_result(l):
    c = l.get("message", {}).get("content")
    return l.get("type") == "user" and isinstance(c, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in c)


def has_text(l):
    return l.get("type") == "assistant" and any(b["type"] == "text" for b in l["message"]["content"])


def has_tool_use(l):
    return l.get("type") == "assistant" and any(b["type"] == "tool_use" for b in l["message"]["content"])


def closing(l):
    return "_meta" in l and ("closing_stdin" in l["_meta"] or "exit_code" in l["_meta"])


def meta(name, key):
    return [l["_meta"] for l in fx(name) if "_meta" in l and key in l["_meta"]]


def metas(name, key=None, **kw):
    return [j(l, **kw) for l in fx(name) if "_meta" in l and not closing(l) and (key is None or key in l["_meta"])]


def reqs(name, tool=None, n=None):
    """can_use_tool requests of a fixture, each followed by the driver's answer."""
    lines, out = fx(name), []
    for i, l in enumerate(lines):
        if l.get("type") == "control_request" and l["request"].get("subtype") == "can_use_tool" and (tool is None or l["request"]["tool_name"] == tool):
            out.append(j(l))
            for m in lines[i + 1:]:
                if "_sent" in m and m["_sent"].get("type") == "control_response" and m["_sent"]["response"].get("request_id") == l["request_id"]:
                    out.append(j(m["_sent"]))
                    break
    return out if n is None else out[:2 * n]


def results(name):
    return [l for l in fx(name) if l.get("type") == "result"]


def cost(names):
    total = 0.0
    for n in names:
        last = {}
        for r in results(n):
            last[r.get("session_id")] = r.get("total_cost_usd") or 0.0
        total += sum(last.values())
    return total


def after_request(name, subtype, nth=0):
    """A predicate that is true from the nth host request of that subtype on."""
    seen = {"n": -1}

    def pred(l):
        if "_sent" in l and l["_sent"].get("type") == "control_request" and l["_sent"]["request"]["subtype"] == subtype:
            seen["n"] += 1
        return seen["n"] >= nth
    return pred
