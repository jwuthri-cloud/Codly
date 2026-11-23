"""Helpers shared by the r7_* scripts (part 5). Times are time stamps of the recording, as in part 4 (r6_lib)."""
import hashlib
import json
import os
import shutil
import subprocess

from drv import OUT, CLAUDE, clean_env
from r6_lib import raw


def line_times(s, since, pred):
    """Time stamps (seconds since the spawn) of every stdout line at or after `since` that matches."""
    return [r["t"] for r in raw(s) if r["dir"] == "recv" and r["t"] >= since and isinstance(r["msg"], dict) and pred(r["msg"])]


def last_sent(s):
    """Time stamp of the last line the host wrote."""
    t = [r["t"] for r in raw(s) if r["dir"] == "send"]
    return t[-1] if t else 0.0


def is_tool_use(name=None, helper=False):
    return lambda e: e.get("type") == "assistant" and bool(e.get("parent_tool_use_id")) == helper and any(
        b.get("type") == "tool_use" and (name is None or b.get("name") == name) for b in e["message"]["content"])


def is_tool_result(helper=False):
    return lambda e: e.get("type") == "user" and bool(e.get("parent_tool_use_id")) == helper and isinstance(e["message"]["content"], list) and any(
        isinstance(b, dict) and b.get("type") == "tool_result" for b in e["message"]["content"])


def tool_results(s, start=0, helper=False):
    """The tool results (content cut to 200 characters) of the main thread (helper=False) or of helpers."""
    with s.cv:
        ev = s.events[start:]
    out = []
    for e in ev:
        if is_tool_result(helper)(e):
            for b in e["message"]["content"]:
                if isinstance(b, dict) and b.get("type") == "tool_result":
                    c = b.get("content")
                    out.append({"is_error": bool(b.get("is_error")), "content": (c if isinstance(c, str) else json.dumps(c))[:200]})
    return out


def home(name):
    """A new, empty throw-away HOME under the capture folder (no login, no file of the user's). What an earlier run of
    the same scenario left there is deleted first: it is this script's own folder."""
    h = os.path.join(OUT, "work", name + "-home")
    shutil.rmtree(h, ignore_errors=True)
    os.makedirs(os.path.join(h, ".claude"))
    return h


def files_of(*roots, skip=("projects", "shell-snapshots", "statsig", "todos", "session-env", "file-history", "debug", "plans", "paste-cache", "telemetry")):
    """{path: sha1} of the files under these folders (the CLI's transcripts and caches are left out: `skip`)."""
    out = {}
    for root in roots:
        for d, dirs, names in os.walk(root):
            dirs[:] = [x for x in dirs if x not in skip and x != ".git"]
            for n in names:
                p = os.path.join(d, n)
                try:
                    out[p] = hashlib.sha1(open(p, "rb").read()).hexdigest()
                except OSError:
                    pass
    return out


def changed(before, after, base):
    """The files that are new or changed between two `files_of` results, relative to `base`."""
    return sorted(os.path.relpath(p, base) for p in after if before.get(p) != after[p])


def read_json(path):
    try:
        return json.load(open(path))
    except (OSError, ValueError):
        return None


def project_entry(home_dir, cwd):
    """The entry of this folder in the throw-away HOME's ~/.claude.json (`projects` keyed by the folder's path)."""
    cfg = read_json(os.path.join(home_dir, ".claude.json")) or {}
    projects = cfg.get("projects", {})
    for k in (cwd, os.path.realpath(cwd)):
        if k in projects:
            return projects[k]
    return None


def queued_notifications(sid):
    """What the session's own transcript (~/.claude/projects/<folder>/<sid>.jsonl, written by the CLI) says about
    task notifications that waited in the CLI's queue: how many were put in it, how many were handed to a model
    request of a running turn (`queued_command` attachments) and how many were taken out without that. Counts only:
    the transcript itself has machine data and is never copied."""
    import glob
    paths = glob.glob(os.path.join(os.path.expanduser("~"), ".claude", "projects", "*", sid + ".jsonl"))
    if len(paths) != 1:
        return {"transcript_found": False}
    out = {"transcript_found": True, "enqueued": 0, "attached_to_a_running_turn": 0, "removed": 0, "dequeued": 0}
    for line in open(paths[0]):
        try:
            o = json.loads(line)
        except ValueError:
            continue
        is_note = lambda text: isinstance(text, str) and text.startswith("<task-notification>")
        if o.get("type") == "queue-operation" and is_note(o.get("content")):
            key = {"enqueue": "enqueued", "remove": "removed", "dequeue": "dequeued"}.get(o.get("operation"))
            if key:
                out[key] += 1
        a = o.get("attachment") or {}
        if o.get("type") == "attachment" and a.get("type") == "queued_command" and is_note(a.get("prompt")):
            out["attached_to_a_running_turn"] += 1
    return out


def claude_cli(args, cwd, env):
    """Run a plain `claude` command (not a session), for set-up steps such as `claude mcp add`. Returns the result."""
    return subprocess.run([CLAUDE] + args, cwd=cwd, env=clean_env() | env, capture_output=True, text=True, timeout=60)
