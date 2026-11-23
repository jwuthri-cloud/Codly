"""Helpers shared by the r6_* scripts (part 4).

Times: the driver's reader thread stamps every stdout line when it reads it, and every line it writes when it writes
it (`t` of the raw recording, seconds since the spawn). `stamps()` reads those. A wait such as `wait_results()` polls
every 0.2 s, so a time measured around it is the poll, not the CLI: no note of part 4 is measured that way.
"""
import json
import subprocess
import threading
import time


def raw(s):
    """The recording so far, parsed. The driver writes and flushes each line while it holds `s.cv`, so a read under
    the same lock never sees half a line."""
    with s.cv:
        lines = open(s.raw.name).read().splitlines()
    return [json.loads(l) for l in lines]


def sent_at(s, subtype):
    """When the host wrote its last control request of that subtype."""
    t = [r["t"] for r in raw(s) if r["dir"] == "send" and r["msg"].get("type") == "control_request" and r["msg"]["request"]["subtype"] == subtype]
    return t[-1] if t else None


def stamps(s, since, **preds):
    """For each named predicate: seconds from `since` to the first stdout line at or after `since` that matches
    (None when there is none)."""
    out = dict.fromkeys(preds)
    for r in raw(s):
        if r["dir"] != "recv" or r["t"] < since or not isinstance(r["msg"], dict):
            continue
        for k, p in preds.items():
            if out[k] is None and p(r["msg"]):
                out[k] = round(r["t"] - since, 3)
    return out


def is_result(e):
    return e.get("type") == "result"


def is_init(e):
    return e.get("type") == "system" and e.get("subtype") == "init"


def system(subtype, **fields):
    """A predicate for a system line of that subtype whose fields have these values."""
    return lambda e: e.get("type") == "system" and e.get("subtype") == subtype and all(e.get(k) == v for k, v in fields.items())


def answer_to(s):
    """A predicate for the answer to the host's last control request."""
    rid = "host-%d" % s.n_req
    return lambda e: e.get("type") == "control_response" and e.get("response", {}).get("request_id") == rid


def kinds(s, start=0):
    """The stdout lines from event index `start` on, as short words (no stream events, counters or status lines)."""
    with s.cv:
        ev = s.events[start:]
    out = []
    for e in ev:
        t, sub = e.get("type"), e.get("subtype")
        if t in ("stream_event", "rate_limit_event") or sub in ("thinking_tokens", "commands_changed", "status"):
            continue
        k = t + ("/" + sub if sub else "")
        if sub == "task_updated":
            k += " " + json.dumps(e.get("patch", {}).get("status") or sorted(e.get("patch", {})))
        elif sub == "task_notification":
            k += " " + json.dumps(e.get("status"))
        elif sub == "background_tasks_changed":
            k += " %d listed" % len(e.get("tasks", []))
        elif t == "control_response":
            k += " " + e.get("response", {}).get("subtype", "")
        if t in ("assistant", "user") and e.get("parent_tool_use_id"):
            k += " (helper)"
        out.append(k)
    return out


def tool_call(s, name, helper, start=0, timeout=60):
    """Wait for a complete assistant line with a tool_use of that tool, written by a helper (helper=True) or by the
    main thread (helper=False)."""
    return s.wait_for(lambda e: e.get("type") == "assistant" and bool(e.get("parent_tool_use_id")) == helper and any(
        b.get("type") == "tool_use" and b.get("name") == name for b in e["message"]["content"]), timeout, start)


def wait_alive(procs, timeout=20):
    """Wait until a process of this pattern (an r5_lib.Procs) runs. A tool call's `tool_use` line can come seconds
    before its process exists (the session's first shell start is slow), so a scenario that stops a running command
    waits for the process, not for the line."""
    end = time.time() + timeout
    while time.time() < end:
        if procs.alive():
            return True
        time.sleep(0.1)
    return False


def note_exit(s):
    """Write a note at the moment the process is gone (Session.close notes the exit code 0.3 s after the fact)."""
    def wait():
        s.p.wait()
        try:
            s.note(process_exited=True)
        except ValueError:  # the recording is already closed
            pass
    threading.Thread(target=wait, daemon=True).start()


def command_of(pid):
    return subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()


def parent_of(pid):
    return subprocess.run(["ps", "-o", "ppid=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()


def is_alive(pid):
    return subprocess.run(["kill", "-0", str(pid)], capture_output=True).returncode == 0


def signal(pid, sig):
    return subprocess.run(["kill", "-" + sig, str(pid)], capture_output=True).returncode == 0
